use crate::model::{CpuInfo, CpuTopology, SensorReading};
use crate::platform;
use std::time::Instant;
use sysinfo::{CpuRefreshKind, MINIMUM_CPU_UPDATE_INTERVAL, System};

pub fn collect() -> CpuInfo {
    let Ok(mut sys) = System::new() else {
        return CpuInfo::default();
    };
    // sysinfo 的使用率是两次 refresh 之间的差值：先取一次基线，
    // 平台侧的慢查询（WMI / sysctl）正好填在等待间隔里。
    sys.refresh_cpu_specifics(CpuRefreshKind::everything());
    let primed = Instant::now();
    let details = platform::cpu_details();
    let wait = MINIMUM_CPU_UPDATE_INTERVAL.saturating_sub(primed.elapsed());
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
    sys.refresh_cpu_specifics(CpuRefreshKind::everything());

    let cpus = sys.cpus();
    let logical_cores = cpus.len() as u32;
    let physical_cores = System::physical_core_count()
        .map(|n| n as u32)
        .unwrap_or(logical_cores)
        .max(1);
    let usage_per_core: Vec<f32> = cpus.iter().map(|c| c.usage()).collect();
    let usage_overall = overall_usage(sys.global_cpu_usage(), &usage_per_core);

    let first = cpus.first();
    let vendor = first.map(|c| c.vendor_id().to_string()).unwrap_or_default();
    let brand = first
        .map(|c| c.brand().trim().to_string())
        .filter(|b| !b.is_empty())
        .or(details.brand.clone())
        .unwrap_or_else(|| "Unknown CPU".to_string());

    let freqs: Vec<u64> = cpus
        .iter()
        .map(|c| c.frequency())
        .filter(|f| *f > 0)
        .collect();
    let observed_max_mhz = freqs.iter().copied().max().unwrap_or(0);
    let observed_avg_mhz = if freqs.is_empty() {
        0
    } else {
        freqs.iter().sum::<u64>() / freqs.len() as u64
    };
    let mhz = |v: u64| v.saturating_mul(1_000_000);
    let current_frequency_hz = if details.live_frequency {
        mhz(observed_avg_mhz)
    } else {
        0
    };
    let max_frequency_hz = details
        .max_hz
        .unwrap_or_else(|| mhz(observed_max_mhz))
        .max(current_frequency_hz);
    let base_frequency_hz = details.base_hz.unwrap_or(0);

    let sockets = details.sockets.unwrap_or(1).max(1);
    let topology = CpuTopology {
        sockets,
        p_cores: details.p_cores,
        e_cores: details.e_cores,
        numa_nodes: details.numa_nodes.unwrap_or(1),
    };

    CpuInfo {
        vendor,
        brand,
        arch: std::env::consts::ARCH.to_string(),
        physical_cores,
        logical_cores,
        base_frequency_hz,
        max_frequency_hz,
        current_frequency_hz,
        cache_l1_bytes: details.l1,
        cache_l2_bytes: details.l2,
        cache_l3_bytes: details.l3,
        features: details.features,
        virtualization: details.virtualization,
        usage_per_core,
        usage_overall,
        temperature_c: None,
        topology: Some(topology),
    }
}

/// sysinfo 的 global 使用率对 P/E 非对称架构做了加权，优先使用；异常时退回各核平均。
pub(crate) fn overall_usage(global: f32, per_core: &[f32]) -> f32 {
    if global.is_finite() && global > 0.0 {
        global
    } else if per_core.is_empty() {
        0.0
    } else {
        per_core.iter().sum::<f32>() / per_core.len() as f32
    }
}

/// 从传感器读数里挑出 CPU 温度：只认明确属于 CPU / SoC 的通道，宁缺毋滥，取其中最高值。
pub fn cpu_temperature(sensors: &[SensorReading]) -> Option<f32> {
    const KEYS: [&str; 6] = ["cpu", "package", "tdie", "tctl", "soc", "core"];
    sensors
        .iter()
        .filter(|s| s.kind == "temperature" && s.value.is_finite())
        .filter(|s| {
            let l = s.label.to_ascii_lowercase();
            KEYS.iter().any(|k| l.contains(k))
        })
        .map(|s| s.value)
        .reduce(f32::max)
}

/// 用传感器读数补上 CPU 温度。
pub fn enrich_with_sensors(cpu: &mut CpuInfo, sensors: &[SensorReading]) {
    if let Some(t) = cpu_temperature(sensors) {
        cpu.temperature_c = Some(t);
    }
}
