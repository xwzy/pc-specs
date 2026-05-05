use crate::model::{InterfaceTick, MonitorTick, SensorReading};
use crate::modules;
use crate::state::{MonitorSys, SharedSys};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

pub const MONITOR_TICK_EVENT: &str = "monitor://tick";

/// task 退出时（正常 break / panic unwind）会触发 Drop，把 alive 标记翻成 false。
/// `start_monitor` 用这个标记判断"slot 残留但 task 已死"，避免被幂等卡住。
struct AliveGuard(Arc<AtomicBool>);
impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub fn spawn_monitor(
    app: AppHandle,
    shared: Arc<SharedSys>,
    monitor: Arc<MonitorSys>,
    interval_ms: u64,
    stop: Arc<Notify>,
    alive: Arc<AtomicBool>,
) {
    alive.store(true, Ordering::SeqCst);
    tokio::spawn(async move {
        // 把 alive 标志的清零放到 RAII guard 里，无论是 select! break 还是任意
        // sample/emit 出 panic，guard.drop 都会清零。
        let _alive_guard = AliveGuard(alive);

        let interval_ms = interval_ms.max(500);
        let interval = Duration::from_millis(interval_ms);
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // skip first immediate tick

        // sensors 在 Windows WMI / macOS system_profiler 下采集成本较高（>50ms），
        // 不需要每 500ms 都拉。每 ~5 秒缓存一次，monitor tick 复用。
        let mut sensors_cache: Vec<SensorReading> = Vec::new();
        let mut sensors_last: Option<Instant> = None;
        const SENSORS_TTL: Duration = Duration::from_secs(5);

        loop {
            tokio::select! {
                _ = stop.notified() => {
                    tracing::info!("monitor stopped");
                    break;
                }
                _ = ticker.tick() => {
                    let need_refresh = sensors_last
                        .map(|t| t.elapsed() >= SENSORS_TTL)
                        .unwrap_or(true);
                    if need_refresh {
                        sensors_cache = modules::sensors::collect(&shared)
                            .into_iter()
                            .filter(|s| s.kind == "temperature")
                            .collect();
                        sensors_last = Some(Instant::now());
                    }
                    let tick = sample(&shared, &monitor, &sensors_cache);
                    if let Err(e) = app.emit(MONITOR_TICK_EVENT, &tick) {
                        tracing::warn!("emit failed: {e}");
                    }
                    // 顺便驱动托盘的实时文字 / tooltip 更新。开销 < 100us。
                    crate::tray::on_tick(&app, &tick);
                }
            }
        }
    });
}

fn sample(
    shared: &Arc<SharedSys>,
    monitor: &Arc<MonitorSys>,
    sensors_cache: &[SensorReading],
) -> MonitorTick {
    // 计算与上一次 sample 的真实 elapsed 时间。第一次为 None 时，
    // 速率类字段返回 0（避免把累计值误算成 BPS）。
    let now = Instant::now();
    let elapsed_secs = {
        let mut last = monitor.last_sample_at.lock();
        let prev = *last;
        *last = Some(now);
        prev.map(|t| now.saturating_duration_since(t).as_secs_f64())
            .unwrap_or(0.0)
    };
    let has_prev = elapsed_secs > 0.05;

    let (cpu_overall, cpu_per_core, mem_used, mem_total) = {
        let mut sys = shared.system.lock();
        sys.refresh_cpu_specifics(CpuRefreshKind::everything().without_frequency());
        sys.refresh_memory_specifics(MemoryRefreshKind::everything());
        let cpus = sys.cpus();
        let per: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
        let g = sys.global_cpu_usage();
        let overall = if g.is_finite() && g > 0.0 {
            g
        } else if per.is_empty() {
            0.0
        } else {
            per.iter().sum::<f32>() / per.len() as f32
        };
        (overall, per, sys.used_memory(), sys.total_memory())
    };

    // 网卡：使用 monitor 独立的 Networks 实例，refresh() 后 received() 是
    // 自上次 refresh（即上一次 monitor tick）以来的累计 bytes —— 严格匹配 elapsed。
    // 同时构建 per_interface 列表，让 Network 页能拿到每张网卡的实时 ↑↓。
    let (rx, tx, per_interface) = {
        let mut nets = monitor.networks.lock();
        nets.refresh();
        let mut rx = 0u64;
        let mut tx = 0u64;
        let mut per: Vec<InterfaceTick> = Vec::with_capacity(nets.iter().count());
        for (name, d) in nets.iter() {
            let r_delta = d.received();
            let w_delta = d.transmitted();
            rx = rx.saturating_add(r_delta);
            tx = tx.saturating_add(w_delta);
            let (r_bps, w_bps) = if has_prev {
                (
                    ((r_delta as f64) / elapsed_secs).round() as u64,
                    ((w_delta as f64) / elapsed_secs).round() as u64,
                )
            } else {
                (0, 0)
            };
            per.push(InterfaceTick {
                name: name.to_string(),
                rx_bps: r_bps,
                tx_bps: w_bps,
            });
        }
        (rx, tx, per)
    };
    let net_rx_bps = if has_prev {
        ((rx as f64) / elapsed_secs).round() as u64
    } else {
        0
    };
    let net_tx_bps = if has_prev {
        ((tx as f64) / elapsed_secs).round() as u64
    } else {
        0
    };

    let (disk_r, disk_w) = if has_prev {
        sample_disk_io(monitor, elapsed_secs)
    } else {
        // 首次 tick 仅做"建立基线"的 refresh，不返回伪造的 BPS。
        prime_disk_io(monitor);
        (0, 0)
    };

    MonitorTick {
        timestamp: modules::now_ms(),
        cpu_overall,
        cpu_per_core,
        mem_used_bytes: mem_used,
        mem_total_bytes: mem_total,
        net_rx_bps,
        net_tx_bps,
        disk_read_bps: disk_r,
        disk_write_bps: disk_w,
        gpu_utilizations: Vec::new(),
        temperatures: sensors_cache.to_vec(),
        per_interface,
    }
}

/// 第一次 tick 只刷新一次状态，不计算 BPS（缺少 elapsed 基线）。
fn prime_disk_io(monitor: &Arc<MonitorSys>) {
    let raw = read_platform_diskstats();
    if raw.is_empty() {
        return;
    }
    let mut prev = monitor.diskstats_prev.lock();
    for (dev, rb, wb) in raw {
        prev.insert(dev, (rb, wb));
    }
}

/// 跨平台磁盘 IO 聚合（按 BPS 返回）。
///
/// - Linux：解析 /proc/diskstats，与 sysinfo refresh 节奏解耦，最精确。
/// - macOS：调用 `iostat -d -K -I` 拿系统启动以来的累计 KiB。
/// - Windows：通过 WMI `Win32_PerfRawData_PerfDisk_PhysicalDisk` 累计字节做差。
///
/// 注：sysinfo 0.32 的 Disk 没有 usage() 接口，所以这里走平台 API。
fn sample_disk_io(monitor: &Arc<MonitorSys>, elapsed_secs: f64) -> (u64, u64) {
    let raw = read_platform_diskstats();
    if raw.is_empty() {
        let _ = monitor;
        return (0, 0);
    }
    let mut prev = monitor.diskstats_prev.lock();
    let mut total_r = 0u64;
    let mut total_w = 0u64;
    for (dev, rb, wb) in &raw {
        let prior = prev.get(dev).copied().unwrap_or((0u64, 0u64));
        total_r = total_r.saturating_add(rb.saturating_sub(prior.0));
        total_w = total_w.saturating_add(wb.saturating_sub(prior.1));
    }
    for (dev, rb, wb) in raw {
        prev.insert(dev, (rb, wb));
    }
    let r = ((total_r as f64) / elapsed_secs).round() as u64;
    let w = ((total_w as f64) / elapsed_secs).round() as u64;
    (r, w)
}

#[cfg(target_os = "linux")]
pub(crate) fn read_platform_diskstats() -> Vec<(String, u64, u64)> {
    read_diskstats()
}

#[cfg(target_os = "windows")]
pub(crate) fn read_platform_diskstats() -> Vec<(String, u64, u64)> {
    read_perf_disk_bytes()
}

#[cfg(target_os = "macos")]
pub(crate) fn read_platform_diskstats() -> Vec<(String, u64, u64)> {
    read_macos_iostat()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) fn read_platform_diskstats() -> Vec<(String, u64, u64)> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn read_diskstats() -> Vec<(String, u64, u64)> {
    let content = match std::fs::read_to_string("/proc/diskstats") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 14 {
            continue;
        }
        let dev = fields[2].to_string();
        // 跳过 loop / ram 等伪设备，保留 sd*/nvme*/mmcblk*/dm-* 等真实块设备。
        // dm-* 是 LVM 上的逻辑卷，会和底层 sd* 重复，但 OS 实际写入两边都计数；
        // 跳过 dm-* 防双计。
        if dev.starts_with("loop") || dev.starts_with("ram") || dev.starts_with("dm-") {
            continue;
        }
        let sectors_read: u64 = fields[5].parse().unwrap_or(0);
        let sectors_written: u64 = fields[9].parse().unwrap_or(0);
        out.push((dev, sectors_read * 512, sectors_written * 512));
    }
    out
}

/// Windows: 通过 WMI Win32_PerfRawData_PerfDisk_PhysicalDisk 拿每个物理盘的累计 bytes。
/// 返回 (device, read_bytes, written_bytes)。我们只取 `_Total` 之外的真实磁盘行。
#[cfg(target_os = "windows")]
fn read_perf_disk_bytes() -> Vec<(String, u64, u64)> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Perf {
        name: Option<String>,
        disk_read_bytes_persec: Option<u64>,
        disk_write_bytes_persec: Option<u64>,
    }
    let Ok(com) = COMLibrary::new() else { return Vec::new() };
    let Ok(conn) = WMIConnection::new(com) else { return Vec::new() };
    // PerfRawData 的 *_Persec 字段在 raw 形式下是「累计 bytes」（不是真的 per second）。
    let rows: Vec<Perf> = conn
        .raw_query(
            "SELECT Name, DiskReadBytesPersec, DiskWriteBytesPersec FROM Win32_PerfRawData_PerfDisk_PhysicalDisk",
        )
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            let name = r.name?;
            // _Total 行是所有盘合计，会与单盘累加重复，跳过。
            if name == "_Total" || name.is_empty() {
                return None;
            }
            Some((
                name,
                r.disk_read_bytes_persec.unwrap_or(0),
                r.disk_write_bytes_persec.unwrap_or(0),
            ))
        })
        .collect()
}

/// macOS: 调用 `iostat -d -K -I disk0 disk1 …` 拿到自系统启动以来的 KiB 累计。
/// `-I` 标志输出 cumulative；`-K` 强制 KiB 单位；不带计数即"快照一次"。
/// 该命令开销 ~30ms，1Hz 下可接受。
#[cfg(target_os = "macos")]
fn read_macos_iostat() -> Vec<(String, u64, u64)> {
    use std::process::Command;
    // 先列出所有 diskN（系统盘 + 容器/外接），最多 8 个。
    let out = match Command::new("iostat").args(["-d", "-K", "-I"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    // iostat 输出格式（无 -n 时）：
    //          disk0           disk2           disk3
    //    KB/t  xfrs    MB     KB/t  xfrs    MB     ...
    //   16.62 32145  522.4   ...
    // 一行 header 是设备名（空格分隔），第二行是字段，第三行是数值。
    let mut lines = s.lines();
    let header = match lines.next() {
        Some(l) => l,
        None => return Vec::new(),
    };
    let _ = lines.next(); // skip "KB/t xfrs MB ..." line
    let data_line = match lines.next() {
        Some(l) => l,
        None => return Vec::new(),
    };
    let devs: Vec<&str> = header.split_whitespace().collect();
    let nums: Vec<&str> = data_line.split_whitespace().collect();
    // 每盘 3 列：KB/t（KB per transfer），xfrs（次数），MB（cumulative MB）
    // 因此 nums.len() 应该 = 3 * devs.len()
    if nums.len() != devs.len() * 3 {
        return Vec::new();
    }
    let mut out_vec = Vec::new();
    for (i, dev) in devs.iter().enumerate() {
        let mb_str = nums[i * 3 + 2];
        let mb: f64 = mb_str.parse().unwrap_or(0.0);
        // iostat 不区分 read / write，只给"transfers"和"MB transferred"。
        // 把累计 MB 平均分到 read 和 write —— 这是近似，但 Storage 页和 Monitor
        // 的 disk 曲线只展示 read+write 之和或趋势，平分对总量无影响。
        // 如果要严格区分，可以改用 IOKit IOBlockStorageDriverStatistics，
        // 但代价是引入 Objective-C 桥接。
        let bytes = (mb * 1024.0 * 1024.0) as u64;
        let half = bytes / 2;
        out_vec.push((dev.to_string(), half, bytes - half));
    }
    out_vec
}
