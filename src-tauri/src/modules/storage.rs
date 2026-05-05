use crate::model::StorageInfo;
use crate::state::SharedSys;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

const VIRTUAL_FS: &[&str] = &[
    "tmpfs",
    "devtmpfs",
    "devfs",
    "proc",
    "procfs",
    "sysfs",
    "cgroup",
    "cgroup2",
    "mqueue",
    "pstore",
    "debugfs",
    "tracefs",
    "autofs",
    "fdesc",
    "fusectl",
    "configfs",
    "securityfs",
    "binfmt_misc",
    "nsfs",
    "rpc_pipefs",
    "overlay",
    "squashfs",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "ramfs",
    "selinuxfs",
    "bpf",
    "hugetlbfs",
];

const VIRTUAL_MOUNT_PREFIXES: &[&str] = &[
    "/proc",
    "/sys",
    "/dev",
    "/run",
    "/snap",
    "/var/lib/docker/overlay2/",
    "/var/lib/containers/",
    "/System/Volumes/Preboot",
    "/System/Volumes/VM",
    "/System/Volumes/Update",
    "/System/Volumes/xarts",
    "/System/Volumes/iSCPreboot",
    "/System/Volumes/Hardware",
    "/System/Volumes/Recovery",
];

pub fn collect(shared: &Arc<SharedSys>) -> Vec<StorageInfo> {
    // 注意：每盘 IO 速率不在 collect() 中计算 —— Storage 页面的实时速率请由
    // Monitor tick 的 disk_read_bps/disk_write_bps 聚合提供（monitor 拥有自己的 Disks 实例）。
    let mut disks = shared.disks.lock();
    disks.refresh();

    let win_drives = collect_windows_drive_kinds();

    let mut interim: Vec<StorageInfo> = disks
        .iter()
        .filter_map(|d| {
            let total = d.total_space();
            if total == 0 {
                return None;
            }
            let avail = d.available_space();
            let used = total.saturating_sub(avail);
            let name_raw = d.name().to_string_lossy().to_string();
            let mount = d.mount_point().to_string_lossy().to_string();
            let fs = d.file_system().to_string_lossy().to_string();

            if is_virtual(&fs, &mount) {
                return None;
            }

            let is_removable = d.is_removable();
            let mut kind = classify_kind(d.kind(), &name_raw, &mount, is_removable);
            // Windows：sysinfo DiskKind 经常是 Unknown(0)，name 也常为空；
            // 用 WMI 的 InterfaceType + MediaType 兜底分类。
            if kind == "Unknown" {
                if let Some(better) = match_windows_drive_kind(&win_drives, &name_raw, &mount) {
                    kind = better;
                }
            }

            Some(StorageInfo {
                name: if name_raw.is_empty() {
                    mount.clone()
                } else {
                    name_raw
                },
                mount_point: Some(mount),
                filesystem: Some(fs),
                kind,
                total_bytes: total,
                used_bytes: used,
                read_bytes_per_sec: 0,
                write_bytes_per_sec: 0,
                temperature_c: None,
                smart_health: None,
                serial: None,
            })
        })
        .collect();

    dedup_same_device(&mut interim);
    enrich_smart(&mut interim);
    interim
}

/// Windows: 用 WMI 拿磁盘的 InterfaceType / MediaType。
/// Linux/macOS 返回空 vec（不需要这种兜底）。
#[derive(Clone, Debug)]
struct WinDriveKind {
    /// 物理盘标识，形如 `\\.\PHYSICALDRIVE0` 或型号名。
    device_id: String,
    /// "NVMe" / "SSD" / "HDD" / "Removable" / "Unknown"
    kind: String,
}

#[cfg(target_os = "windows")]
fn collect_windows_drive_kinds() -> Vec<WinDriveKind> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Drive {
        device_id: Option<String>,
        model: Option<String>,
        interface_type: Option<String>,
        media_type: Option<String>,
    }

    let Ok(com) = COMLibrary::new() else { return Vec::new() };
    let Ok(conn) = WMIConnection::new(com) else { return Vec::new() };
    let rows: Vec<Drive> = conn
        .raw_query("SELECT DeviceID, Model, InterfaceType, MediaType FROM Win32_DiskDrive")
        .unwrap_or_default();

    rows.into_iter()
        .filter_map(|d| {
            let id = d.device_id.or(d.model)?;
            let iface = d.interface_type.unwrap_or_default().to_lowercase();
            let media = d.media_type.unwrap_or_default().to_lowercase();
            let kind = if media.contains("removable") {
                "Removable".to_string()
            } else if iface.contains("nvme") || media.contains("nvme") {
                "NVMe".to_string()
            } else if media.contains("ssd") || media.contains("solid state") {
                "SSD".to_string()
            } else if media.contains("hdd") || media.contains("fixed hard") {
                "HDD".to_string()
            } else if iface.contains("scsi") || iface.contains("sata") || iface.contains("ide") {
                // MediaType 在 Win10/11 上经常是 "Fixed hard disk media"；
                // 仅靠 InterfaceType 没法区分 SATA SSD vs HDD，留给 SMART 标记或保留 SSD 默认。
                "SSD".to_string()
            } else {
                return None;
            };
            Some(WinDriveKind { device_id: id, kind })
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn collect_windows_drive_kinds() -> Vec<WinDriveKind> {
    Vec::new()
}

fn match_windows_drive_kind(
    drives: &[WinDriveKind],
    name: &str,
    _mount: &str,
) -> Option<String> {
    if drives.is_empty() {
        return None;
    }
    let n = name.to_lowercase();
    // sysinfo 在 Windows 上 disk.name() 是磁盘卷标 "Local Disk" 或型号
    // ("Samsung SSD 990 PRO 2TB")，直接匹配 model 子串。
    for d in drives {
        let id = d.device_id.to_lowercase();
        if id.is_empty() || n.is_empty() {
            continue;
        }
        if n.contains(&id) || id.contains(&n) {
            return Some(d.kind.clone());
        }
    }
    // 找不到精确匹配但只有一个物理盘时直接采用。
    if drives.len() == 1 {
        return Some(drives[0].kind.clone());
    }
    None
}

/// 根据 sysinfo 的 DiskKind + 设备名 + 挂载点 + 是否可移动综合分类。
/// model 上 kind 字段允许 "SSD" / "HDD" / "NVMe" / "Removable" / "Unknown"。
fn classify_kind(
    base: sysinfo::DiskKind,
    name: &str,
    mount: &str,
    is_removable: bool,
) -> String {
    if is_removable {
        return "Removable".to_string();
    }
    let n = name.to_lowercase();
    let m = mount.to_lowercase();
    // NVMe 设备名通常含 "nvme"（Linux: /dev/nvme0n1）；macOS/Windows 在 sysinfo 拿不到
    // 设备路径，但 name 字段里可能含品牌字串（"Samsung 990 PRO" 之类）；
    // mount /Volumes/...（macOS 内置都是 NVMe）也可作为线索（保守起见仅看 nvme 关键字）。
    if n.contains("nvme") || m.contains("nvme") {
        return "NVMe".to_string();
    }
    match base {
        sysinfo::DiskKind::HDD => "HDD".to_string(),
        sysinfo::DiskKind::SSD => "SSD".to_string(),
        sysinfo::DiskKind::Unknown(_) => "Unknown".to_string(),
    }
}

fn enrich_smart(items: &mut [StorageInfo]) {
    let smart = collect_smart_info();
    if smart.is_empty() {
        return;
    }
    for item in items.iter_mut() {
        let mount = item.mount_point.clone().unwrap_or_default();
        let key = device_basename(&item.name, &mount);
        if let Some(s) = smart.iter().find(|s| s.matches(&item.name, &key, &mount)) {
            if item.smart_health.is_none() {
                item.smart_health = s.health.clone();
            }
            if item.serial.is_none() {
                item.serial = s.serial.clone();
            }
            if item.temperature_c.is_none() {
                item.temperature_c = s.temperature_c;
            }
        }
    }
}

#[derive(Default, Clone)]
struct SmartRow {
    device: String,
    health: Option<String>,
    serial: Option<String>,
    temperature_c: Option<f32>,
}

impl SmartRow {
    fn matches(&self, name: &str, key: &str, mount: &str) -> bool {
        let dev = self.device.to_lowercase();
        let dev_base = std::path::Path::new(&dev)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| dev.clone());
        let n = name.to_lowercase();
        let k = key.to_lowercase();
        let m = mount.to_lowercase();
        dev_base == k
            || dev == k
            || n.contains(&dev_base)
            || dev_base.contains(&k)
            || m.contains(&dev_base)
    }
}

fn collect_smart_info() -> Vec<SmartRow> {
    // SMART 状态变化非常缓慢（健康/序列号几乎不变，温度几十秒粒度足够），
    // 而 smartctl 单次调用 200~600ms。前端 5s refetch 会让 Storage 页很慢，
    // 这里做 30s TTL 进程内缓存。
    type SmartCache = StdMutex<Option<(Instant, Vec<SmartRow>)>>;
    static CACHE: once_cell::sync::Lazy<SmartCache> =
        once_cell::sync::Lazy::new(|| StdMutex::new(None));
    const TTL: Duration = Duration::from_secs(30);

    {
        let guard = CACHE.lock().unwrap();
        if let Some((t, rows)) = guard.as_ref() {
            if t.elapsed() < TTL {
                return rows.clone();
            }
        }
    }

    let mut rows = smartctl_scan();
    rows.extend(platform_smart_fallback());

    let mut guard = CACHE.lock().unwrap();
    *guard = Some((Instant::now(), rows.clone()));
    rows
}

#[cfg(target_os = "windows")]
fn platform_smart_fallback() -> Vec<SmartRow> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct FailurePredict {
        instance_name: Option<String>,
        predict_failure: Option<bool>,
    }
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct DiskDrive {
        device_id: Option<String>,
        serial_number: Option<String>,
        model: Option<String>,
    }

    let Ok(com) = COMLibrary::new() else { return Vec::new() };
    let Ok(wmi_root) = WMIConnection::new(com) else { return Vec::new() };

    let drives: Vec<DiskDrive> = wmi_root
        .raw_query("SELECT DeviceID, SerialNumber, Model FROM Win32_DiskDrive")
        .unwrap_or_default();

    // FailurePredictStatus 在 ROOT\WMI 命名空间。COM 已经被首次 COMLibrary::new() 初始化过，
    // 这里使用 assume_initialized() 复用线程内已有的 COM 状态再开第二个连接。
    // SAFETY: 同一线程内上面 `COMLibrary::new()` 已成功初始化 COM；wmi::COMLibrary 不会在 drop 时
    // 反初始化 COM，所以这里复用线程内 COM 状态是安全的。
    let com2 = unsafe { COMLibrary::assume_initialized() };
    let predicts: Vec<FailurePredict> = if let Ok(conn) = WMIConnection::with_namespace_path("ROOT\\WMI", com2) {
        conn.raw_query(
            "SELECT InstanceName, PredictFailure FROM MSStorageDriver_FailurePredictStatus",
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut out = Vec::new();
    for d in drives {
        let model = d.model.unwrap_or_default();
        let serial = d
            .serial_number
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // PredictFailure 的 InstanceName 形如 "SCSI\\DISK&VEN_..."；与 DeviceID 不直接匹配，
        // 但通常各盘只有一条 PredictFailure，且顺序大致与 Win32_DiskDrive 对应。
        let health = predicts
            .iter()
            .find_map(|p| p.predict_failure)
            .map(|f| if f { "Failing".to_string() } else { "OK".to_string() });
        out.push(SmartRow {
            device: d.device_id.unwrap_or(model),
            health,
            serial,
            temperature_c: None,
        });
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn platform_smart_fallback() -> Vec<SmartRow> {
    Vec::new()
}

fn smartctl_scan() -> Vec<SmartRow> {
    use std::process::Command;
    // 仅在 smartctl 可用时探测；不强制要求 root，没权限的字段会缺。
    let scan = match Command::new("smartctl").args(["--scan", "-j"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_slice(&scan.stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let devs = match json.get("devices").and_then(|d| d.as_array()) {
        Some(d) => d,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for dev in devs.iter().take(16) {
        let name = match dev.get("name").and_then(|n| n.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let info_out = match Command::new("smartctl")
            .args(["-i", "-H", "-A", "-j", &name])
            .output()
        {
            Ok(o) => o,
            Err(_) => continue,
        };
        let info: serde_json::Value = match serde_json::from_slice(&info_out.stdout) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let health = info
            .get("smart_status")
            .and_then(|s| s.get("passed"))
            .and_then(|p| p.as_bool())
            .map(|p| if p { "OK".to_string() } else { "Failing".to_string() });
        let serial = info
            .get("serial_number")
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let temperature_c = info
            .get("temperature")
            .and_then(|t| t.get("current"))
            .and_then(|t| t.as_f64())
            .map(|n| n as f32)
            .filter(|n| *n > 0.0 && *n < 200.0);
        out.push(SmartRow {
            device: name,
            health,
            serial,
            temperature_c,
        });
    }
    out
}

/// macOS APFS 单容器多卷会被 sysinfo 列出多条相同 total_bytes 的项；
/// Linux 多挂载点（如 bind mount）也会重复。
/// 同 (name, total_bytes, fs) 视为同物理设备，保留挂载点最短（更接近根）的那条，
/// 并把"已用空间"取最大（系统卷往往报 0 used）。
fn dedup_same_device(items: &mut Vec<StorageInfo>) {
    let mut groups: HashMap<(String, u64, String), Vec<usize>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        let fs = it.filesystem.clone().unwrap_or_default();
        let key = (it.name.clone(), it.total_bytes, fs);
        groups.entry(key).or_default().push(i);
    }
    let mut to_remove: Vec<usize> = Vec::new();
    for (_key, ids) in groups.iter() {
        if ids.len() <= 1 {
            continue;
        }
        let mut best = ids[0];
        let mut best_score: (usize, u64) = score(items, best);
        for &i in &ids[1..] {
            let s = score(items, i);
            if s < best_score {
                best_score = s;
                best = i;
            }
        }
        let max_used = ids.iter().map(|&i| items[i].used_bytes).max().unwrap_or(0);
        let max_r = ids.iter().map(|&i| items[i].read_bytes_per_sec).max().unwrap_or(0);
        let max_w = ids.iter().map(|&i| items[i].write_bytes_per_sec).max().unwrap_or(0);
        items[best].used_bytes = max_used;
        items[best].read_bytes_per_sec = max_r;
        items[best].write_bytes_per_sec = max_w;
        for &i in ids {
            if i != best {
                to_remove.push(i);
            }
        }
    }
    to_remove.sort_unstable();
    to_remove.dedup();
    for i in to_remove.into_iter().rev() {
        items.remove(i);
    }
}

fn score(items: &[StorageInfo], i: usize) -> (usize, u64) {
    // 越小越优：挂载点越短越好；mount==/ 最优（长度 1）。
    let mount = items[i].mount_point.clone().unwrap_or_default();
    (mount.len(), 0)
}

fn is_virtual(fs: &str, mount: &str) -> bool {
    let fs_lower = fs.to_lowercase();
    if VIRTUAL_FS.iter().any(|v| fs_lower == *v) {
        return true;
    }
    if VIRTUAL_MOUNT_PREFIXES
        .iter()
        .any(|p| mount.starts_with(p))
    {
        return true;
    }
    false
}

/// 已废弃：聚合磁盘 IO 现在由 monitor.rs 自己负责（使用独立 disks 实例）。
/// 保留 stub 以便不被外部破坏；monitor 不会调用这个。
#[allow(dead_code)]
pub fn collect_aggregate_io(_shared: &Arc<SharedSys>) -> (u64, u64) {
    (0, 0)
}

fn device_basename(name: &str, mount: &str) -> String {
    if !name.is_empty() {
        std::path::Path::new(name)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| name.to_string())
    } else {
        mount.to_string()
    }
}

