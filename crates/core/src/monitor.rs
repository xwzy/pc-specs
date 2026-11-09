//! 实时采样。
//!
//! 速率 = 两次采样之间的计数差 ÷ 真实间隔。因此每个 `Sampler` 持有自己的 sysinfo
//! 基线，互不干扰：应用的监控线程和 HTTP 接口各用一个，谁也不会"偷走"对方的增量。

use crate::model::{DiskTick, GpuTick, InterfaceTick, MonitorTick, ProcessInfo, SensorReading};
use crate::modules::{cpu, gpu, network, sensors, storage};
use crate::platform;
use crate::process_net::NetSampler;
use crate::util::now_ms;
use std::collections::HashMap;
use std::time::{Duration, Instant};
#[cfg(not(target_os = "macos"))]
use sysinfo::Networks;
use sysinfo::{
    Components, CpuRefreshKind, DiskRefreshKind, Disks, Gpus, MemoryRefreshKind,
    ProcessRefreshKind, ProcessesToUpdate, System,
};

/// 温度传感器的读取成本在 Windows（WMI）和 macOS（HID）上都不低，没必要每个 tick 都读。
pub const TEMPERATURE_TTL: Duration = Duration::from_secs(3);
/// 重新枚举磁盘列表的周期（发现热插拔的 U 盘 / 外置硬盘）。
const DISK_RELIST: Duration = Duration::from_secs(30);
/// 间隔短于此值时不换算速率（避免把一点点字节除以极小的时间放大成噪声）。
const MIN_RATE_INTERVAL: f64 = 0.05;
/// 超过此时间没有采样，基线就过期了，需要重新建立。
const STALE_AFTER: Duration = Duration::from_secs(3);

/// 一个接口这个采样周期里收发的字节数。
type NetDelta = (String, u64, u64);

#[cfg(target_os = "macos")]
#[derive(Default)]
struct NativeNet {
    reader: crate::platform::macos_net::Reader,
    /// 接口名 → 上次读到的累计收发字节，以及最近一次看到它的轮次。
    previous: HashMap<String, (u64, u64, u64)>,
    round: u64,
    scratch: Vec<crate::platform::macos_net::Counters>,
}

#[cfg(target_os = "macos")]
impl NativeNet {
    /// 各接口自上次读取以来的字节增量。第一次见到的接口增量记 0。
    fn deltas(&mut self) -> Option<Vec<NetDelta>> {
        self.scratch.clear();
        if !self.reader.read(&mut self.scratch) {
            return None;
        }
        self.round += 1;
        let round = self.round;
        let mut out = Vec::with_capacity(self.scratch.len());
        for counters in self.scratch.drain(..) {
            let (rx_delta, tx_delta) = match self.previous.get_mut(counters.name.as_str()) {
                Some(last) => {
                    // 计数器被重置（接口重建）时当作没有增量，不能算出一个巨大的数。
                    let delta = (
                        counters.rx_bytes.saturating_sub(last.0),
                        counters.tx_bytes.saturating_sub(last.1),
                    );
                    *last = (counters.rx_bytes, counters.tx_bytes, round);
                    delta
                }
                None => {
                    self.previous.insert(
                        counters.name.clone(),
                        (counters.rx_bytes, counters.tx_bytes, round),
                    );
                    (0, 0)
                }
            };
            out.push((counters.name, rx_delta, tx_delta));
        }
        // 这一轮没出现的接口已经不存在了。
        self.previous.retain(|_, last| last.2 == round);
        Some(out)
    }
}

/// 磁盘 IO 要读的卷：同一块物理盘的多个卷共享 IO 计数，只需要读其中一个。
#[cfg(target_os = "macos")]
struct DiskPlan {
    /// 在 `Disks` 列表里的位置。
    index: usize,
    /// 这块物理盘上挂载点最短的卷。
    mount_point: String,
}

pub struct Sampler {
    system: Option<System>,
    /// macOS 上 CPU 使用率自己算（见 `platform::macos_cpu`），sysinfo 的读数只在这条路读不到时兜底。
    #[cfg(target_os = "macos")]
    cpu_load: crate::platform::macos_cpu::CpuLoad,
    #[cfg(not(target_os = "macos"))]
    networks: Option<Networks>,
    #[cfg(target_os = "macos")]
    native_net: NativeNet,
    disks: Option<Disks>,
    #[cfg(target_os = "macos")]
    disk_plan: Vec<DiskPlan>,
    gpus: Option<Gpus>,
    components: Option<Components>,
    /// 接口名 → 类型，名字只分类一次。
    net_kinds: HashMap<String, &'static str>,
    temperatures: Vec<SensorReading>,
    /// 风扇、功率等：和温度同一个节奏读。
    other_sensors: Vec<SensorReading>,
    thermal_pressure: Option<u8>,
    temperatures_at: Option<Instant>,
    /// 温度传感器多久读一次；`None` 表示现在没人需要温度，一律不读。
    temperature_every: Option<Duration>,
    disks_listed_at: Instant,
    last_at: Option<Instant>,
    /// 上一次的累计换入 / 换出字节数，用来算换页速率。
    swapped_before: Option<(u64, u64)>,
}

/// 1 / 5 / 15 分钟平均负载。Windows 没有这个概念（sysinfo 在那里恒为 0），不提供。
fn load_average() -> Option<[f32; 3]> {
    if cfg!(windows) {
        return None;
    }
    let load = System::load_average().ok()?;
    Some([load.one as f32, load.five as f32, load.fifteen as f32])
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    /// 创建并建立基线。第一次 `sample()` 应当在至少 `MINIMUM_CPU_UPDATE_INTERVAL`（约 200ms）之后调用，
    /// 这样 CPU 使用率与各项速率都有真实的时间窗口。
    pub fn new() -> Self {
        let mut system = System::new().ok();
        if let Some(s) = system.as_mut() {
            s.refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage());
        }
        let now = Instant::now();
        #[cfg(target_os = "macos")]
        let cpu_load = {
            let mut load = crate::platform::macos_cpu::CpuLoad::default();
            let _ = load.sample();
            load
        };
        #[cfg(target_os = "macos")]
        let native_net = {
            // 先读一遍当基线，第一个采样才有增量可算。
            let mut native = NativeNet::default();
            let _ = native.deltas();
            native
        };
        #[cfg(not(target_os = "macos"))]
        let networks = Networks::new_with_refreshed_list().ok();
        let disks = Disks::new_with_refreshed_list().ok();
        Self {
            system,
            #[cfg(target_os = "macos")]
            cpu_load,
            #[cfg(not(target_os = "macos"))]
            networks,
            #[cfg(target_os = "macos")]
            native_net,
            #[cfg(target_os = "macos")]
            disk_plan: disks.as_ref().map(plan_disks).unwrap_or_default(),
            disks,
            gpus: Gpus::new_with_refreshed_list().ok(),
            components: Components::new_with_refreshed_list().ok(),
            net_kinds: HashMap::new(),
            temperatures: Vec::new(),
            other_sensors: Vec::new(),
            thermal_pressure: None,
            temperatures_at: None,
            temperature_every: Some(TEMPERATURE_TTL),
            disks_listed_at: now,
            last_at: Some(now),
            swapped_before: None,
        }
    }

    /// 设置温度传感器的读取周期。`None` 表示现在没人需要温度，不再读取（传感器读起来很重：
    /// macOS 上每次要和 hidd 来回通信，Windows 上要走 WMI）。需要的时候下一次采样会立刻补读。
    pub fn set_temperature_interval(&mut self, every: Option<Duration>) {
        self.temperature_every = every;
        if every.is_none() {
            // 不再需要了：缓存的旧读数不能留着，下次再要的时候重新读。
            self.temperatures = Vec::new();
            self.other_sensors = Vec::new();
            self.thermal_pressure = None;
            self.temperatures_at = None;
        }
    }

    /// 采一次样。速率相对于上一次调用（首次相对于 `new()`）。
    pub fn sample(&mut self) -> MonitorTick {
        let now = Instant::now();
        let elapsed = self
            .last_at
            .replace(now)
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f64());
        let rate = |bytes: u64| -> u64 {
            if elapsed >= MIN_RATE_INTERVAL {
                (bytes as f64 / elapsed).round() as u64
            } else {
                0
            }
        };

        let mut tick = MonitorTick {
            timestamp: now_ms(),
            elapsed_secs: elapsed,
            ..Default::default()
        };

        if let Some(sys) = self.system.as_mut() {
            sys.refresh_cpu_specifics(CpuRefreshKind::nothing().with_usage());
            sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram().with_swap());
            tick.cpu_per_core = sys.cpus().iter().map(|c| c.usage()).collect();
            tick.cpu_overall = cpu::overall_usage(sys.global_cpu_usage(), &tick.cpu_per_core);
            tick.mem_used_bytes = sys.used_memory();
            tick.mem_total_bytes = sys.total_memory();
            tick.swap_used_bytes = sys.used_swap();
            tick.swap_total_bytes = sys.total_swap();
        }
        #[cfg(target_os = "macos")]
        if let Some(usage) = self.cpu_load.sample() {
            tick.cpu_per_core = usage.per_core;
            tick.cpu_overall = usage.overall;
        }
        tick.memory = platform::memory_breakdown().map(|mut memory| {
            // 累计量换算成速率；第一次没有上一次可比，记 0。
            if let Some((before_in, before_out)) = self
                .swapped_before
                .replace((memory.swapped_in_bytes, memory.swapped_out_bytes))
            {
                memory.swap_in_bps = rate(memory.swapped_in_bytes.saturating_sub(before_in));
                memory.swap_out_bps = rate(memory.swapped_out_bytes.saturating_sub(before_out));
            }
            // 几部分加起来要正好是总量：各平台统计口径之外的零头（固件保留之类）算作空闲。
            let accounted = memory.app_bytes
                + memory.wired_bytes
                + memory.compressed_bytes
                + memory.cached_bytes;
            if tick.mem_total_bytes > 0 {
                memory.free_bytes = tick.mem_total_bytes.saturating_sub(accounted);
            }
            memory
        });
        tick.load_average = load_average();

        if let Some(interfaces) = self.read_networks() {
            let (mut rx_counted, mut tx_counted) = (0u64, 0u64);
            let (mut rx_any, mut tx_any) = (0u64, 0u64);
            tick.per_interface.reserve(interfaces.len());
            for (name, rx, tx) in interfaces {
                let kind = match self.net_kinds.get(name.as_str()) {
                    Some(kind) => *kind,
                    None => {
                        let kind = network::classify_interface(&name, None);
                        self.net_kinds.insert(name.clone(), kind);
                        kind
                    }
                };
                if kind != "loopback" {
                    rx_any = rx_any.saturating_add(rx);
                    tx_any = tx_any.saturating_add(tx);
                    // 虚拟接口（docker 网桥、VPN 隧道、veth…）上的流量会在物理网卡上再出现一次，不计入总量。
                    if kind != "virtual" {
                        rx_counted = rx_counted.saturating_add(rx);
                        tx_counted = tx_counted.saturating_add(tx);
                    }
                }
                tick.per_interface.push(InterfaceTick {
                    name,
                    rx_bps: rate(rx),
                    tx_bps: rate(tx),
                });
            }
            // 极端情况下所有接口都被判成虚拟的，退回"全部非回环"，总比显示 0 好。
            let (rx, tx) = if rx_counted + tx_counted == 0 {
                (rx_any, tx_any)
            } else {
                (rx_counted, tx_counted)
            };
            tick.net_rx_bps = rate(rx);
            tick.net_tx_bps = rate(tx);
            tick.per_interface.sort_by(|a, b| a.name.cmp(&b.name));
        }

        if let Some(disks) = self.disks.as_mut() {
            let relist = self.disks_listed_at.elapsed() >= DISK_RELIST;
            let io = DiskRefreshKind::nothing().with_io_usage();
            #[cfg(target_os = "macos")]
            let per_disk: Vec<DiskTick> = {
                // sysinfo 的 `Disks::refresh_specifics` 在 macOS 上每次都把全部卷重新枚举一遍
                // （每个卷要问文件系统十来项属性，再逐卷向 IOKit 查统计），一秒一次太重。
                // 平时只刷新每块物理盘的代表卷（同一块盘的卷共享 IO 计数），整张列表隔 30 秒重列一次。
                if relist {
                    disks.refresh_specifics(true, io);
                    self.disks_listed_at = Instant::now();
                    self.disk_plan = plan_disks(disks);
                } else {
                    for plan in &self.disk_plan {
                        if let Some(disk) = disks.get_mut(plan.index) {
                            disk.refresh_specifics(io);
                        }
                    }
                }
                let disks = &*disks;
                self.disk_plan
                    .iter()
                    .filter_map(|plan| {
                        let usage = disks.get(plan.index)?.usage();
                        Some(DiskTick {
                            mount_point: plan.mount_point.clone(),
                            read_bps: rate(usage.read_bytes),
                            write_bps: rate(usage.written_bytes),
                        })
                    })
                    .collect()
            };
            #[cfg(not(target_os = "macos"))]
            let per_disk: Vec<DiskTick> = {
                disks.refresh_specifics(relist, io);
                if relist {
                    self.disks_listed_at = Instant::now();
                }
                // 同一块物理盘的多个卷共享 IO 计数，按物理键去重后再汇总。
                let mut groups: HashMap<String, DiskTick> = HashMap::new();
                for d in disks.iter() {
                    let mount = d.mount_point().to_string_lossy().into_owned();
                    if storage::is_virtual(&d.file_system().to_string_lossy(), &mount)
                        || d.total_space() == 0
                    {
                        continue;
                    }
                    let key = storage::physical_key(&d.name().to_string_lossy(), d.id(), &mount);
                    let usage = d.usage();
                    let entry = groups.entry(key).or_insert_with(|| DiskTick {
                        mount_point: mount.clone(),
                        read_bps: rate(usage.read_bytes),
                        write_bps: rate(usage.written_bytes),
                    });
                    if mount.len() < entry.mount_point.len() {
                        entry.mount_point = mount;
                    }
                }
                let mut per_disk: Vec<DiskTick> = groups.into_values().collect();
                per_disk.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
                per_disk
            };
            tick.disk_read_bps = per_disk.iter().map(|d| d.read_bps).sum();
            tick.disk_write_bps = per_disk.iter().map(|d| d.write_bps).sum();
            tick.per_disk = per_disk;
        }

        if let Some(gpus) = self.gpus.as_mut() {
            gpus.refresh(false);
            tick.gpus = gpus
                .iter()
                .map(|g| GpuTick {
                    name: g.model().unwrap_or("GPU").trim().to_string(),
                    utilization: g.usage(),
                    vram_used_bytes: g.used_memory(),
                    vram_total_bytes: g.total_memory(),
                })
                .filter(|g| !gpu::is_virtual_adapter(&g.name))
                .collect();
        }

        if let Some(every) = self.temperature_every {
            if self.temperatures_at.is_none_or(|t| t.elapsed() >= every) {
                let mut temperatures = self.components.as_mut().map_or_else(Vec::new, |c| {
                    c.refresh(false);
                    sensors::temperatures(c)
                });
                // 平台自己的传感器里也有温度（macOS 的 CPU / GPU 逐核温度），并进温度列表，其余归「其他」。
                let (platform_temperatures, others): (Vec<_>, Vec<_>) =
                    crate::platform::live_sensors()
                        .into_iter()
                        .partition(|r| r.kind == "temperature");
                temperatures.extend(platform_temperatures);
                self.temperatures = temperatures;
                self.other_sensors = others;
                self.thermal_pressure = platform::thermal_pressure();
                self.temperatures_at = Some(Instant::now());
            }
            tick.temperatures = self.temperatures.clone();
            tick.other_sensors = self.other_sensors.clone();
            tick.thermal_pressure = self.thermal_pressure;
        }

        tick
    }

    /// 各接口自上次读取以来收发的字节数。
    #[cfg(target_os = "macos")]
    fn read_networks(&mut self) -> Option<Vec<NetDelta>> {
        self.native_net.deltas()
    }

    #[cfg(not(target_os = "macos"))]
    fn read_networks(&mut self) -> Option<Vec<NetDelta>> {
        let nets = self.networks.as_mut()?;
        nets.refresh(true);
        Some(
            nets.iter()
                .map(|(name, d)| {
                    (
                        name.to_string_lossy().into_owned(),
                        d.received(),
                        d.transmitted(),
                    )
                })
                .collect(),
        )
    }

    /// 需要"此刻"读数、又不能保证采样器一直在跑时使用（例如 HTTP 接口）。
    /// 基线过期就先重新建立，再等一个最小窗口采样，避免返回"过去十分钟的平均值"。
    pub fn sample_now(&mut self) -> MonitorTick {
        if self.last_at.is_none_or(|t| t.elapsed() > STALE_AFTER) {
            self.sample();
        }
        // 新建采样器的首个请求也要等基线成熟，否则 CPU / IO 会返回无效的首帧。
        let wait = sample_delay(self.last_at, Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        self.sample()
    }
}

/// 挑出要读 IO 的卷：同一块物理盘的多个卷共享 IO 计数，每块盘只取一个代表，
/// 展示用挂载点最短（最接近根）的那个卷。
#[cfg(target_os = "macos")]
fn plan_disks(disks: &Disks) -> Vec<DiskPlan> {
    let mut groups: HashMap<String, DiskPlan> = HashMap::new();
    for (index, d) in disks.iter().enumerate() {
        let mount = d.mount_point().to_string_lossy().into_owned();
        if storage::is_virtual(&d.file_system().to_string_lossy(), &mount) || d.total_space() == 0 {
            continue;
        }
        let key = storage::physical_key(&d.name().to_string_lossy(), d.id(), &mount);
        match groups.get_mut(&key) {
            Some(plan) => {
                if mount.len() < plan.mount_point.len() {
                    plan.mount_point = mount;
                }
            }
            None => {
                groups.insert(
                    key,
                    DiskPlan {
                        index,
                        mount_point: mount,
                    },
                );
            }
        }
    }
    let mut plan: Vec<DiskPlan> = groups.into_values().collect();
    plan.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    plan
}

fn sample_delay(last_at: Option<Instant>, now: Instant) -> Duration {
    let window = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(100);
    window.saturating_sub(last_at.map_or(Duration::ZERO, |at| now.saturating_duration_since(at)))
}

/// 进程列表采样（任务管理器式的轻量视图）。
pub struct ProcessSampler {
    system: Option<System>,
    last_at: Option<Instant>,
    /// 按进程统计网络流量。要另外读一遍套接字表，所以只在有人看的时候才开（见 [`Self::set_network`]）。
    net: Option<NetSampler>,
    /// 最近一次 `sample` 有没有拿到网络流量：刚开始统计、还在建基线的那一帧没有，不能当成「零流量」。
    net_ready: bool,
}

impl Default for ProcessSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSampler {
    pub fn new() -> Self {
        let mut s = Self {
            system: System::new().ok(),
            last_at: None,
            net: None,
            net_ready: false,
        };
        // 建立基线：进程的 CPU / IO 同样是两次刷新之间的差值。
        s.refresh();
        s.last_at = Some(Instant::now());
        s
    }

    fn refresh(&mut self) {
        if let Some(sys) = self.system.as_mut() {
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                // Linux 上线程也算 "task"，不关心线程就关掉，成本差一个数量级。
                ProcessRefreshKind::nothing()
                    .with_cpu()
                    .with_memory()
                    .with_disk_usage()
                    .without_tasks(),
            );
        }
    }

    /// 开关每个进程的网络流量统计。系统不支持（[`crate::process_net::SUPPORTED`]）时开了也没有数据。
    /// 打开时马上读一遍套接字作为基线，下一次 `sample` 就能给出速率。
    pub fn set_network(&mut self, on: bool) {
        if on == self.net.is_some() {
            return;
        }
        self.net_ready = false;
        self.net = on.then(|| {
            let mut net = NetSampler::new();
            let _ = net.sample();
            net
        });
    }

    /// 最近一次 [`Self::sample`] 里的网络流量是不是有效的。
    pub fn network_ready(&self) -> bool {
        self.net_ready
    }

    /// 全部进程，未排序。`cpu_percent` 以单核满载为 100%，多线程进程可以超过 100。
    pub fn sample(&mut self) -> Vec<ProcessInfo> {
        let now = Instant::now();
        let elapsed = self
            .last_at
            .replace(now)
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f64());
        self.refresh();
        let traffic = self.net.as_mut().and_then(NetSampler::sample);
        self.net_ready = traffic.is_some();
        let rate = |bytes: u64| -> u64 {
            if elapsed >= MIN_RATE_INTERVAL {
                (bytes as f64 / elapsed).round() as u64
            } else {
                0
            }
        };
        let Some(sys) = self.system.as_ref() else {
            return Vec::new();
        };
        sys.processes()
            .values()
            .map(|p| {
                let io = p.disk_usage();
                let pid = p.pid().as_u32();
                let (net_rx_bps, net_tx_bps) = traffic
                    .as_ref()
                    .and_then(|traffic| traffic.get(&pid))
                    .copied()
                    .unwrap_or_default();
                ProcessInfo {
                    pid,
                    name: p.name().to_string_lossy().into_owned(),
                    cpu_percent: p.cpu_usage(),
                    // 系统自带的任务管理器怎么算就怎么算，读不到的进程才用 sysinfo 的 RSS。
                    mem_bytes: platform::process_memory(pid)
                        .filter(|bytes| *bytes > 0)
                        .unwrap_or_else(|| p.memory()),
                    read_bps: rate(io.read_bytes),
                    write_bps: rate(io.written_bytes),
                    net_rx_bps,
                    net_tx_bps,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_waits_for_a_valid_cpu_window() {
        let now = Instant::now();
        let window = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(100);
        assert_eq!(sample_delay(None, now), window);
        assert_eq!(sample_delay(Some(now), now), window);
        assert_eq!(
            sample_delay(Some(now), now + Duration::from_millis(50)),
            window - Duration::from_millis(50)
        );
        assert_eq!(
            sample_delay(Some(now), now + Duration::from_secs(1)),
            Duration::ZERO
        );
    }
}
