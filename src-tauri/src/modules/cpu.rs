use crate::model::{CpuInfo, CpuTopology, SensorReading};
use crate::state::SharedSys;
use std::sync::Arc;
use sysinfo::CpuRefreshKind;

pub fn collect(shared: &Arc<SharedSys>) -> CpuInfo {
    let mut sys = shared.system.lock();
    sys.refresh_cpu_specifics(CpuRefreshKind::everything());

    let cpus = sys.cpus();
    let logical_cores = cpus.len() as u32;
    let physical_cores = sys
        .physical_core_count()
        .unwrap_or(logical_cores as usize) as u32;
    let usage_per_core: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
    // sysinfo 0.32 提供 global_cpu_usage()——它做加权处理而不是简单平均，
    // 在 P/E core 不对称架构（M-series / Alder Lake）上更准确。
    let usage_overall = {
        let g = sys.global_cpu_usage();
        if g.is_finite() && g > 0.0 {
            g
        } else if usage_per_core.is_empty() {
            0.0
        } else {
            usage_per_core.iter().sum::<f32>() / usage_per_core.len() as f32
        }
    };

    let first = cpus.first();
    let vendor = first.map(|c| c.vendor_id().to_string()).unwrap_or_default();
    let brand_sysinfo = first.map(|c| c.brand().to_string()).unwrap_or_default();
    let brand = if brand_sysinfo.is_empty() {
        crate::platform::cpu_brand_fallback().unwrap_or_else(|| "Unknown CPU".to_string())
    } else {
        brand_sysinfo
    };
    let current_freq_mhz = first.map(|c| c.frequency()).unwrap_or(0);
    let max_freq_mhz = cpus.iter().map(|c| c.frequency()).max().unwrap_or(current_freq_mhz);
    drop(sys);

    let arch = std::env::consts::ARCH.to_string();
    let features = read_features();
    let virtualization = features
        .iter()
        .any(|f| matches!(f.as_str(), "vmx" | "svm" | "hypervisor"));
    let (l1, l2, l3) = read_caches();
    let sockets = crate::platform::cpu_sockets().unwrap_or(1).max(1);
    let topology = read_topology(physical_cores, logical_cores, sockets);
    let (base_mhz, max_mhz) = derive_frequencies(current_freq_mhz, max_freq_mhz);

    CpuInfo {
        vendor,
        brand,
        arch,
        physical_cores,
        logical_cores,
        base_frequency_hz: base_mhz.saturating_mul(1_000_000),
        max_frequency_hz: max_mhz.saturating_mul(1_000_000),
        current_frequency_hz: current_freq_mhz.saturating_mul(1_000_000),
        cache_l1_bytes: l1,
        cache_l2_bytes: l2,
        cache_l3_bytes: l3,
        features,
        virtualization: Some(virtualization),
        usage_per_core,
        usage_overall,
        temperature_c: None,
        topology,
    }
}

pub fn enrich_with_sensors(cpu: &mut CpuInfo, sensors: &[SensorReading]) {
    if cpu.temperature_c.is_some() {
        return;
    }
    let pick = sensors
        .iter()
        .filter(|s| s.kind == "temperature")
        .find(|s| {
            let l = s.label.to_lowercase();
            l.contains("cpu") || l.contains("package") || l.contains("tdie") || l.contains("tctl")
        })
        .or_else(|| sensors.iter().find(|s| s.kind == "temperature"));
    if let Some(s) = pick {
        cpu.temperature_c = Some(s.value);
    }
}

fn derive_frequencies(current_mhz: u64, max_mhz: u64) -> (u64, u64) {
    if max_mhz > current_mhz {
        (current_mhz, max_mhz)
    } else if current_mhz > 0 {
        (current_mhz, current_mhz)
    } else {
        (0, 0)
    }
}

#[cfg(target_os = "linux")]
fn read_features() -> Vec<String> {
    if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in content.lines() {
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim();
                if k == "flags" || k == "Features" {
                    return v.split_whitespace().map(|s| s.to_string()).collect();
                }
            }
        }
    }
    Vec::new()
}

#[cfg(target_os = "macos")]
fn read_features() -> Vec<String> {
    use std::process::Command;
    let mut out = Vec::new();
    let keys = [
        "machdep.cpu.features",
        "machdep.cpu.leaf7_features",
        "machdep.cpu.extfeatures",
    ];
    for k in keys {
        if let Ok(o) = Command::new("sysctl").arg("-n").arg(k).output() {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout);
                out.extend(
                    s.split_whitespace()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_lowercase()),
                );
            }
        }
    }
    if out.is_empty() {
        let arm_keys = ["hw.optional.neon", "hw.optional.arm.FEAT_FP16", "hw.optional.arm.FEAT_DotProd", "hw.optional.arm.FEAT_SVE"];
        let labels = ["neon", "fp16", "dotprod", "sve"];
        for (k, l) in arm_keys.iter().zip(labels.iter()) {
            if let Ok(o) = Command::new("sysctl").arg("-n").arg(k).output() {
                if o.status.success() {
                    let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if v == "1" {
                        out.push((*l).to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(target_os = "windows")]
fn read_features() -> Vec<String> {
    Vec::new()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_features() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn read_caches() -> (Option<u64>, Option<u64>, Option<u64>) {
    use std::process::Command;
    let q = |k: &str| -> Option<u64> {
        let o = Command::new("sysctl").arg("-n").arg(k).output().ok()?;
        if !o.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        s.parse::<u64>().ok().filter(|v| *v > 0)
    };
    (
        q("hw.l1dcachesize").or_else(|| q("hw.l1icachesize")),
        q("hw.l2cachesize"),
        q("hw.l3cachesize"),
    )
}

#[cfg(target_os = "linux")]
fn read_caches() -> (Option<u64>, Option<u64>, Option<u64>) {
    let mut l1: Option<u64> = None;
    let mut l2: Option<u64> = None;
    let mut l3: Option<u64> = None;
    if let Ok(entries) = std::fs::read_dir("/sys/devices/system/cpu/cpu0/cache") {
        for e in entries.flatten() {
            let level_path = e.path().join("level");
            let size_path = e.path().join("size");
            let level = std::fs::read_to_string(&level_path)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());
            let size = std::fs::read_to_string(&size_path)
                .ok()
                .and_then(|s| parse_size_kb(s.trim()));
            match (level, size) {
                (Some(1), Some(v)) => {
                    l1 = Some(l1.unwrap_or(0) + v);
                }
                (Some(2), Some(v)) => l2 = Some(v.max(l2.unwrap_or(0))),
                (Some(3), Some(v)) => l3 = Some(v.max(l3.unwrap_or(0))),
                _ => {}
            }
        }
    }
    (l1, l2, l3)
}

#[cfg(target_os = "linux")]
fn parse_size_kb(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix('K') {
        rest.parse::<u64>().ok().map(|v| v * 1024)
    } else if let Some(rest) = s.strip_suffix('M') {
        rest.parse::<u64>().ok().map(|v| v * 1024 * 1024)
    } else {
        s.parse::<u64>().ok()
    }
}

#[cfg(target_os = "windows")]
fn read_caches() -> (Option<u64>, Option<u64>, Option<u64>) {
    (None, None, None)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_caches() -> (Option<u64>, Option<u64>, Option<u64>) {
    (None, None, None)
}

#[cfg(target_os = "macos")]
fn read_topology(physical: u32, _logical: u32, sockets: u32) -> Option<CpuTopology> {
    use std::process::Command;
    let q = |k: &str| -> Option<u32> {
        let o = Command::new("sysctl").arg("-n").arg(k).output().ok()?;
        if !o.status.success() {
            return None;
        }
        String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok()
    };
    let p = q("hw.perflevel0.physicalcpu");
    let e = q("hw.perflevel1.physicalcpu");
    let topo = CpuTopology {
        sockets,
        p_cores: p,
        e_cores: e,
        numa_nodes: 1,
    };
    Some(if topo.p_cores.is_none() && topo.e_cores.is_none() {
        CpuTopology {
            sockets,
            p_cores: Some(physical),
            e_cores: None,
            numa_nodes: 1,
        }
    } else {
        topo
    })
}

#[cfg(target_os = "linux")]
fn read_topology(_physical: u32, _logical: u32, sockets: u32) -> Option<CpuTopology> {
    let nodes = std::fs::read_dir("/sys/devices/system/node")
        .ok()
        .map(|it| it.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("node")).count() as u32)
        .filter(|n| *n > 0)
        .unwrap_or(1);
    Some(CpuTopology {
        sockets,
        p_cores: None,
        e_cores: None,
        numa_nodes: nodes,
    })
}

#[cfg(target_os = "windows")]
fn read_topology(_physical: u32, _logical: u32, sockets: u32) -> Option<CpuTopology> {
    Some(CpuTopology {
        sockets,
        p_cores: None,
        e_cores: None,
        numa_nodes: 1,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_topology(_physical: u32, _logical: u32, _sockets: u32) -> Option<CpuTopology> {
    None
}
