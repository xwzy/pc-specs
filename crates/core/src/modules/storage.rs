use crate::model::StorageInfo;
use crate::util::which;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use sysinfo::{DiskKind, Disks};

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

/// 磁盘容量 / 类型 / 健康。每盘的实时 IO 速率由 `monitor::Sampler` 提供。
pub fn collect() -> Vec<StorageInfo> {
    let Ok(disks) = Disks::new_with_refreshed_list() else {
        return Vec::new();
    };
    let win_drives = windows_drive_kinds();

    let mut items: Vec<(StorageInfo, Option<u64>)> = disks
        .iter()
        .filter_map(|d| {
            let total = d.total_space();
            if total == 0 {
                return None;
            }
            let name_raw = d.name().to_string_lossy().to_string();
            let mount = d.mount_point().to_string_lossy().to_string();
            let fs = d.file_system().to_string_lossy().to_string();
            if is_virtual(&fs, &mount) {
                return None;
            }
            let mut kind = classify_kind(d.kind(), &name_raw, d.is_removable());
            if kind == "Unknown"
                && let Some(k) = match_windows_drive_kind(&win_drives, &name_raw)
            {
                kind = k;
            }
            let used = total.saturating_sub(d.available_space());
            Some((
                StorageInfo {
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
                    ..Default::default()
                },
                d.id(),
            ))
        })
        .collect();

    dedup_same_device(&mut items);
    let mut items: Vec<StorageInfo> = items.into_iter().map(|(s, _)| s).collect();
    native_enrich(&mut items);
    enrich_smart(&mut items);
    items
}

/// sysinfo 的类型 + 设备名 + 是否可移动综合分类：NVMe / SSD / HDD / Removable / Unknown。
fn classify_kind(base: DiskKind, name: &str, is_removable: bool) -> String {
    if is_removable {
        return "Removable".to_string();
    }
    if name.to_ascii_lowercase().contains("nvme") {
        return "NVMe".to_string();
    }
    match base {
        DiskKind::HDD => "HDD",
        DiskKind::SSD => "SSD",
        DiskKind::Unknown(_) => "Unknown",
    }
    .to_string()
}

pub(crate) fn is_virtual(fs: &str, mount: &str) -> bool {
    let fs = fs.to_ascii_lowercase();
    VIRTUAL_FS.contains(&fs.as_str()) || VIRTUAL_MOUNT_PREFIXES.iter().any(|p| mount.starts_with(p))
}

/// macOS APFS 同一容器的多个卷共用一个设备 id；Linux 的 bind mount 同名同大小。
/// 它们是同一块物理存储：保留挂载点最短（更接近根）的一条，已用空间取最大
/// （只读系统卷往往报告很小的 used）。
fn dedup_same_device(items: &mut Vec<(StorageInfo, Option<u64>)>) {
    let mut groups: HashMap<(String, u64, String, Option<u64>), Vec<usize>> = HashMap::new();
    for (i, (it, id)) in items.iter().enumerate() {
        let key = (
            it.name.clone(),
            it.total_bytes,
            it.filesystem.clone().unwrap_or_default(),
            *id,
        );
        groups.entry(key).or_default().push(i);
    }
    let mut remove: Vec<usize> = Vec::new();
    for ids in groups.values().filter(|ids| ids.len() > 1) {
        let mount_len = |i: usize| items[i].0.mount_point.as_deref().map_or(0, str::len);
        let best = *ids.iter().min_by_key(|&&i| mount_len(i)).unwrap_or(&ids[0]);
        let max_used = ids
            .iter()
            .map(|&i| items[i].0.used_bytes)
            .max()
            .unwrap_or(0);
        items[best].0.used_bytes = max_used;
        remove.extend(ids.iter().copied().filter(|&i| i != best));
    }
    remove.sort_unstable();
    for i in remove.into_iter().rev() {
        items.remove(i);
    }
}

// ----------------------------------------------------------- Windows 磁盘类型

#[derive(Clone, Debug)]
struct WinDriveKind {
    /// 物理盘标识：`\\.\PHYSICALDRIVE0` 或型号名。
    device_id: String,
    kind: String,
}

#[cfg(target_os = "windows")]
fn windows_drive_kinds() -> Vec<WinDriveKind> {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Drive {
        device_id: Option<String>,
        model: Option<String>,
        interface_type: Option<String>,
        media_type: Option<String>,
    }

    let Some(conn) = crate::platform::windows::wmi_cimv2() else {
        return Vec::new();
    };
    let rows: Vec<Drive> = conn
        .raw_query("SELECT DeviceID, Model, InterfaceType, MediaType FROM Win32_DiskDrive")
        .unwrap_or_default();

    rows.into_iter()
        .filter_map(|d| {
            let id = d.device_id.or(d.model)?;
            let iface = d.interface_type.unwrap_or_default().to_lowercase();
            let media = d.media_type.unwrap_or_default().to_lowercase();
            let kind = if media.contains("removable") {
                "Removable"
            } else if iface.contains("nvme") || media.contains("nvme") {
                "NVMe"
            } else if media.contains("ssd") || media.contains("solid state") {
                "SSD"
            } else if media.contains("hdd") || media.contains("fixed hard") {
                "HDD"
            } else if iface.contains("scsi") || iface.contains("sata") || iface.contains("ide") {
                "SSD"
            } else {
                return None;
            };
            Some(WinDriveKind {
                device_id: id,
                kind: kind.to_string(),
            })
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn windows_drive_kinds() -> Vec<WinDriveKind> {
    Vec::new()
}

fn match_windows_drive_kind(drives: &[WinDriveKind], name: &str) -> Option<String> {
    if drives.is_empty() {
        return None;
    }
    let n = name.to_lowercase();
    for d in drives {
        let id = d.device_id.to_lowercase();
        if id.is_empty() || n.is_empty() {
            continue;
        }
        if n.contains(&id) || id.contains(&n) {
            return Some(d.kind.clone());
        }
    }
    // 找不到精确匹配但只有一块物理盘时直接采用。
    (drives.len() == 1).then(|| drives[0].kind.clone())
}

// --------------------------------------------------------- macOS: diskutil

/// macOS：`diskutil info` 给出总线协议（Apple Fabric / PCI-Express / USB / SATA）和
/// 系统自带的 SMART 状态，不依赖 smartctl。
#[cfg(target_os = "macos")]
fn native_enrich(items: &mut [StorageInfo]) {
    let handles: Vec<_> = items
        .iter()
        .map(|it| it.mount_point.clone().unwrap_or_default())
        .map(|mount| {
            std::thread::spawn(move || {
                crate::util::run_stdout_timeout(
                    "/usr/sbin/diskutil",
                    &["info", &mount],
                    Duration::from_secs(6),
                )
            })
        })
        .collect();
    for (it, h) in items.iter_mut().zip(handles) {
        let Some(text) = h.join().ok().flatten() else {
            continue;
        };
        let field = |key: &str| {
            text.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                (k.trim() == key).then(|| v.trim().to_string())
            })
        };
        let protocol = field("Protocol").unwrap_or_default().to_ascii_lowercase();
        if protocol.contains("fabric") || protocol.contains("nvme") || protocol.contains("pci") {
            it.kind = "NVMe".to_string();
        } else if it.kind == "Unknown" && protocol.contains("usb") {
            it.kind = "Removable".to_string();
        }
        if it.smart_health.is_none() {
            it.smart_health = field("SMART Status").and_then(|s| match s.as_str() {
                "Verified" => Some("OK".to_string()),
                "Failing" => Some("Failing".to_string()),
                _ => None,
            });
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn native_enrich(_items: &mut [StorageInfo]) {}

// ------------------------------------------------------------------ SMART

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

fn device_basename(name: &str, mount: &str) -> String {
    if name.is_empty() {
        return mount.to_string();
    }
    std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// SMART 状态变化很慢，而 `smartctl` 单次调用 200~600ms，所以做 30 秒进程内缓存。
fn collect_smart_info() -> Vec<SmartRow> {
    type Cache = Mutex<Option<(Instant, Vec<SmartRow>)>>;
    static CACHE: LazyLock<Cache> = LazyLock::new(|| Mutex::new(None));
    const TTL: Duration = Duration::from_secs(30);

    let mut guard = CACHE.lock();
    if let Some((t, rows)) = guard.as_ref()
        && t.elapsed() < TTL
    {
        return rows.clone();
    }
    let mut rows = smartctl_scan();
    rows.extend(windows_smart_fallback());
    *guard = Some((Instant::now(), rows.clone()));
    rows
}

#[cfg(target_os = "windows")]
fn windows_smart_fallback() -> Vec<SmartRow> {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct FailurePredict {
        predict_failure: Option<bool>,
    }
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct DiskDrive {
        device_id: Option<String>,
        serial_number: Option<String>,
        model: Option<String>,
    }

    let Some(conn) = crate::platform::windows::wmi_cimv2() else {
        return Vec::new();
    };
    let drives: Vec<DiskDrive> = conn
        .raw_query("SELECT DeviceID, SerialNumber, Model FROM Win32_DiskDrive")
        .unwrap_or_default();
    let predicts: Vec<FailurePredict> = crate::platform::windows::wmi_root_wmi()
        .and_then(|c| {
            c.raw_query("SELECT PredictFailure FROM MSStorageDriver_FailurePredictStatus")
                .ok()
        })
        .unwrap_or_default();

    // FailurePredictStatus 的 InstanceName 与 DeviceID 无法直接对应；
    // 仅在两张表行数相同时按顺序对齐，否则不给 health，避免多盘互相张冠李戴。
    let aligned = !drives.is_empty() && drives.len() == predicts.len();
    drives
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            let model = d.model.unwrap_or_default();
            SmartRow {
                device: d.device_id.unwrap_or(model),
                health: aligned
                    .then(|| predicts[i].predict_failure)
                    .flatten()
                    .map(|f| if f { "Failing" } else { "OK" }.to_string()),
                serial: d
                    .serial_number
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                temperature_c: None,
            }
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn windows_smart_fallback() -> Vec<SmartRow> {
    Vec::new()
}

fn smartctl_scan() -> Vec<SmartRow> {
    // 仅在 smartctl 可用时探测；不强制 root，没权限的字段会缺失。
    let path = crate::util::dev_path();
    if which("smartctl", Some(path)).is_none() {
        return Vec::new();
    }
    let run = |args: &[&str]| -> Option<serde_json::Value> {
        let o = crate::util::run_with("smartctl", args, Duration::from_secs(6), Some(path))?;
        serde_json::from_str(&o.stdout).ok()
    };
    let Some(scan) = run(&["--scan", "-j"]) else {
        return Vec::new();
    };
    let Some(devs) = scan.get("devices").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dev in devs.iter().take(16) {
        let Some(name) = dev.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let Some(info) = run(&["-i", "-H", "-A", "-j", name]) else {
            continue;
        };
        out.push(SmartRow {
            device: name.to_string(),
            health: info
                .get("smart_status")
                .and_then(|s| s.get("passed"))
                .and_then(|p| p.as_bool())
                .map(|p| if p { "OK" } else { "Failing" }.to_string()),
            serial: info
                .get("serial_number")
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            temperature_c: info
                .get("temperature")
                .and_then(|t| t.get("current"))
                .and_then(|t| t.as_f64())
                .map(|n| n as f32)
                .filter(|n| *n > 0.0 && *n < 200.0),
        });
    }
    out
}

/// 磁盘所属的物理设备键：同一块盘上的多个分区 / 卷共享它。
///
/// sysinfo 的 IO 计数在 Linux 上按整盘读取、在 macOS 上按容器读取，
/// 所以汇总速率时必须先按此键去重，否则同一块盘会被重复计数。
pub(crate) fn physical_key(name: &str, id: Option<u64>, mount: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        let _ = mount;
        id.map_or_else(|| name.to_string(), |i| format!("id:{i}"))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = (id, mount);
        let base = name.trim_start_matches("/dev/");
        // nvme0n1p2 / mmcblk0p1 → 去掉 `p<数字>`；sda3 → 去掉尾部数字。
        if base.starts_with("nvme") || base.starts_with("mmcblk") || base.starts_with("loop") {
            if let Some((disk, part)) = base.rsplit_once('p')
                && !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
            {
                return disk.to_string();
            }
            return base.to_string();
        }
        if base.starts_with("sd")
            || base.starts_with("vd")
            || base.starts_with("hd")
            || base.starts_with("xvd")
        {
            return base
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .to_string();
        }
        base.to_string()
    }
    #[cfg(target_os = "windows")]
    {
        // Windows 每个卷的计数相互独立，盘符就是最稳定的键（卷标可能重名）。
        let _ = (name, id);
        mount.to_string()
    }
}
