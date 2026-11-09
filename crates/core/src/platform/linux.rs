use super::CpuDetails;
use crate::model::{MemoryBreakdown, MemoryModule, MotherboardInfo, SensorReading};
use crate::util::{clean_dmi, run_stdout_timeout};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

fn read(path: impl AsRef<Path>) -> Option<String> {
    let s = fs::read_to_string(path).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read(path)?.parse().ok()
}

// ---------------------------------------------------------------- CPU

pub fn cpu_details() -> CpuDetails {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();

    let mut brand = None;
    let mut features: Vec<String> = Vec::new();
    let mut packages: HashSet<String> = HashSet::new();
    for line in cpuinfo.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "model name" | "Model Name" | "cpu model" if brand.is_none() => {
                brand = Some(v.trim().to_string());
            }
            "flags" | "Features" if features.is_empty() => {
                features = v.split_whitespace().map(str::to_string).collect();
            }
            "physical id" => {
                packages.insert(v.trim().to_string());
            }
            _ => {}
        }
    }

    let (l1, l2, l3) = read_caches();
    let cpufreq = Path::new("/sys/devices/system/cpu/cpu0/cpufreq");
    let khz = |name: &str| {
        read_u64(cpufreq.join(name))
            .map(|v| v * 1000)
            .filter(|v| *v > 0)
    };
    let (p_cores, e_cores) = hybrid_cores();
    let numa_nodes = fs::read_dir("/sys/devices/system/node")
        .ok()
        .map(|it| {
            it.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.strip_prefix("node")
                        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
                })
                .count() as u32
        })
        .filter(|n| *n > 0);

    let virtualization = if features.is_empty() {
        None
    } else {
        Some(features.iter().any(|f| matches!(f.as_str(), "vmx" | "svm")))
    };

    CpuDetails {
        brand,
        features,
        l1,
        l2,
        l3,
        sockets: (!packages.is_empty()).then_some(packages.len() as u32),
        p_cores,
        e_cores,
        numa_nodes,
        base_hz: khz("base_frequency"),
        max_hz: khz("cpuinfo_max_freq"),
        live_frequency: true,
        virtualization,
    }
}

fn read_caches() -> (Option<u64>, Option<u64>, Option<u64>) {
    let (mut l1, mut l2, mut l3) = (None, None, None);
    let Ok(entries) = fs::read_dir("/sys/devices/system/cpu/cpu0/cache") else {
        return (l1, l2, l3);
    };
    for e in entries.flatten() {
        let dir = e.path();
        let level = read_u64(dir.join("level"));
        let kind = read(dir.join("type")).unwrap_or_default();
        let size = read(dir.join("size")).and_then(|s| parse_cache_size(&s));
        match (level, size) {
            // L1 取数据缓存（指令缓存不计入，与 macOS 口径一致）。
            (Some(1), Some(v)) if kind == "Data" || kind == "Unified" => l1 = Some(v),
            (Some(2), Some(v)) => l2 = Some(v),
            (Some(3), Some(v)) => l3 = Some(v),
            _ => {}
        }
    }
    (l1, l2, l3)
}

/// "32K" / "1024K" / "16M" → 字节
fn parse_cache_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('K') {
        n.parse::<u64>().ok().map(|v| v << 10)
    } else if let Some(n) = s.strip_suffix('M') {
        n.parse::<u64>().ok().map(|v| v << 20)
    } else {
        s.parse().ok()
    }
}

/// "0-7,16,18-19" → [0..7, 16, 18, 19]
fn parse_cpu_list(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in s.trim().split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                    out.extend(a..=b);
                }
            }
            None => out.extend(part.trim().parse::<u32>().ok()),
        }
    }
    out
}

/// 物理核数（同一 package 内 core_id 去重，排除超线程）。
fn physical_core_count(cpus: &[u32]) -> u32 {
    let mut seen = HashSet::new();
    for c in cpus {
        let topo = format!("/sys/devices/system/cpu/cpu{c}/topology");
        let pkg = read(format!("{topo}/physical_package_id")).unwrap_or_default();
        let core = read(format!("{topo}/core_id")).unwrap_or_else(|| c.to_string());
        seen.insert((pkg, core));
    }
    seen.len() as u32
}

/// 识别混合架构：Intel Alder Lake+（cpu_core / cpu_atom PMU）与 ARM big.LITTLE（cpu_capacity）。
fn hybrid_cores() -> (Option<u32>, Option<u32>) {
    let core = read("/sys/devices/cpu_core/cpus").map(|s| parse_cpu_list(&s));
    let atom = read("/sys/devices/cpu_atom/cpus").map(|s| parse_cpu_list(&s));
    if let (Some(p), Some(e)) = (core, atom)
        && !p.is_empty()
        && !e.is_empty()
    {
        return (Some(physical_core_count(&p)), Some(physical_core_count(&e)));
    }

    let mut by_capacity: BTreeMap<u64, u32> = BTreeMap::new();
    let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") else {
        return (None, None);
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let is_cpu = name
            .strip_prefix("cpu")
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        if !is_cpu {
            continue;
        }
        if let Some(cap) = read_u64(e.path().join("cpu_capacity")) {
            *by_capacity.entry(cap).or_default() += 1;
        }
    }
    if by_capacity.len() > 1 {
        let top = by_capacity.values().next_back().copied();
        let rest: u32 = by_capacity.values().sum::<u32>() - top.unwrap_or(0);
        return (top, Some(rest));
    }
    (None, None)
}

// ------------------------------------------------------- 主板 / 固件

pub fn motherboard() -> Option<MotherboardInfo> {
    let dmi = Path::new("/sys/class/dmi/id");
    let get = |name: &str| read(dmi.join(name)).and_then(clean_dmi);

    if dmi.exists() {
        let chassis = get("chassis_type").map(|t| match t.parse::<u16>().unwrap_or(0) {
            3..=7 | 15 | 16 => "Desktop".to_string(),
            8..=11 | 14 => "Laptop".to_string(),
            30..=32 => "Tablet".to_string(),
            12 | 21 => "Mini-PC".to_string(),
            13 => "All-in-One".to_string(),
            17..=20 | 23 | 24 => "Server".to_string(),
            _ => format!("Type-{t}"),
        });
        let info = MotherboardInfo {
            vendor: get("board_vendor").or_else(|| get("sys_vendor")),
            model: get("board_name").or_else(|| get("product_name")),
            version: get("board_version"),
            serial: get("board_serial"),
            bios_vendor: get("bios_vendor"),
            bios_version: get("bios_version"),
            bios_date: get("bios_date"),
            chassis,
        };
        if info.vendor.is_some() || info.model.is_some() || info.bios_version.is_some() {
            return Some(info);
        }
    }

    // 没有 DMI 的 ARM 开发板（树莓派等）：设备树里有型号与序列号。
    let dt = Path::new("/sys/firmware/devicetree/base");
    let model = fs::read(dt.join("model"))
        .ok()
        .map(|b| {
            String::from_utf8_lossy(&b)
                .trim_end_matches('\0')
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty());
    model.map(|m| MotherboardInfo {
        vendor: None,
        model: Some(m),
        serial: fs::read(dt.join("serial-number"))
            .ok()
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .trim_end_matches('\0')
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty()),
        chassis: Some("Board".to_string()),
        ..Default::default()
    })
}

/// 内存条信息。SMBIOS 原始表只有 root 可读，但 systemd-udevd 会把解码结果放进 udev 数据库，
/// 普通用户通过 `udevadm info` 就能读到（systemd ≥ 247）。
pub fn memory_modules() -> Vec<MemoryModule> {
    let Some(text) = run_stdout_timeout(
        "udevadm",
        &["info", "-p", "/sys/devices/virtual/dmi/id"],
        Duration::from_secs(3),
    ) else {
        return Vec::new();
    };

    let mut devices: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("E: MEMORY_DEVICE_") else {
            continue;
        };
        let Some((idx, tail)) = rest.split_once('_') else {
            continue;
        };
        let (Ok(idx), Some((key, value))) = (idx.parse::<u32>(), tail.split_once('=')) else {
            continue;
        };
        devices
            .entry(idx)
            .or_default()
            .insert(key.to_string(), value.trim().to_string());
    }

    devices
        .into_values()
        .filter_map(|d| {
            let size: u64 = d.get("SIZE")?.parse().ok().filter(|s| *s > 0)?;
            let get = |k: &str| d.get(k).cloned().and_then(clean_dmi);
            let speed = d
                .get("CONFIGURED_SPEED_MTS")
                .or_else(|| d.get("SPEED_MTS"))
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|s| *s > 0);
            Some(MemoryModule {
                slot: get("LOCATOR").unwrap_or_else(|| "DIMM".to_string()),
                manufacturer: get("MANUFACTURER"),
                part_number: get("PART_NUMBER"),
                capacity_bytes: size,
                speed_mt_s: speed,
                kind: get("TYPE"),
                form_factor: get("FORM_FACTOR"),
            })
        })
        .collect()
}

// ------------------------------------------------------------ 传感器

/// hwmon 里 sysinfo 不覆盖的通道：风扇 / 电压 / 电流 / 功率（温度已由 sysinfo 提供）。
///
/// 单位参考 Documentation/hwmon/sysfs-interface：
/// `fanN_input` RPM；`inN_input` mV；`currN_input` mA；`powerN_input` µW。
pub fn extra_sensors() -> Vec<SensorReading> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let source = read(dir.join("name"))
            .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
        let Ok(files) = fs::read_dir(&dir) else {
            continue;
        };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some(base) = name.strip_suffix("_input") else {
                continue;
            };
            let Some(raw) = read(f.path()).and_then(|s| s.parse::<f32>().ok()) else {
                continue;
            };
            let label = read(dir.join(format!("{base}_label"))).unwrap_or_else(|| base.to_string());

            let (kind, value, unit) = if base.starts_with("fan") {
                if !(0.0..50_000.0).contains(&raw) {
                    continue;
                }
                ("fan", raw, "RPM")
            } else if base.starts_with("curr") {
                let v = raw / 1000.0;
                if !(-200.0..200.0).contains(&v) {
                    continue;
                }
                ("current", v, "A")
            } else if base.starts_with("power") {
                let v = raw / 1_000_000.0;
                if raw == 0.0 || !(-2000.0..2000.0).contains(&v) {
                    continue;
                }
                ("power", v, "W")
            } else if base.starts_with("in") && base[2..].bytes().all(|b| b.is_ascii_digit()) {
                let v = raw / 1000.0;
                if raw == 0.0 || !(-30.0..30.0).contains(&v) {
                    continue;
                }
                ("voltage", v, "V")
            } else {
                continue;
            };
            out.push(SensorReading {
                source: source.clone(),
                label,
                kind: kind.to_string(),
                value,
                unit: unit.to_string(),
            });
        }
    }
    out
}

/// 内存构成与压力：`/proc/meminfo`、`/proc/vmstat`，内核开了 PSI 时再加 `/proc/pressure/memory`。
pub fn memory_breakdown() -> Option<MemoryBreakdown> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    let vmstat = fs::read_to_string("/proc/vmstat").unwrap_or_default();
    let psi = fs::read_to_string("/proc/pressure/memory").ok();
    super::proc_mem::parse(&meminfo, &vmstat, psi.as_deref())
}

pub fn desktop_env() -> Option<String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|s| !s.is_empty());
    let session = std::env::var("XDG_SESSION_TYPE")
        .ok()
        .filter(|s| !s.is_empty() && s != "tty")
        .map(|s| match s.as_str() {
            "wayland" => "Wayland".to_string(),
            "x11" => "X11".to_string(),
            other => other.to_string(),
        });
    match (desktop, session) {
        (Some(d), Some(s)) => Some(format!("{d} · {s}")),
        (Some(d), None) => Some(d),
        (None, Some(s)) => Some(s),
        _ => None,
    }
}

/// Linux 上 sysinfo 给的 RSS 就是 top / htop 的口径，不需要另外读。
pub fn process_memory(_pid: u32) -> Option<u64> {
    None
}
