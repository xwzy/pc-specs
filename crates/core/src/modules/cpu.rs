use crate::model::{CpuInfo, CpuTopology, SensorReading};
use crate::modules::sensors;
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

/// 整体使用率：优先用 sysinfo 的 global 值（各核使用率的平均），异常时退回自己算的各核平均。
pub(crate) fn overall_usage(global: f32, per_core: &[f32]) -> f32 {
    if global.is_finite() && global > 0.0 {
        global
    } else if per_core.is_empty() {
        0.0
    } else {
        per_core.iter().sum::<f32>() / per_core.len() as f32
    }
}

/// 从传感器读数里挑出 CPU 温度，口径和 iStat Menus / Stats 一致：
///
/// 1. Apple 芯片有逐核传感器时，取它们的平均值；
/// 2. 其他平台取封装 / 控制温度（`Package id 0`、`Tctl`、`Tdie`、`CPU …`），没有再取各核里最高的；
/// 3. Apple 芯片读不到逐核传感器（比如还没收录键表的新芯片）时，退回电源管理芯片的 die 温度，
///    它比核心低，只能当个参考。
///
/// 电池、硬盘、主板这类传感器一律不算：宁缺毋滥。
pub fn cpu_temperature(sensors: &[SensorReading]) -> Option<f32> {
    let readings = || {
        sensors
            .iter()
            .filter(|s| s.kind == "temperature" && s.value.is_finite())
    };
    let mean = |values: Vec<f32>| {
        (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
    };

    let cores: Vec<f32> = readings()
        .filter(|s| sensors::is_cpu_core(&s.label))
        .map(|s| s.value)
        .collect();
    if let Some(avg) = mean(cores) {
        return Some(avg);
    }

    let hottest_matching = |keys: &[&str]| {
        readings()
            .filter(|s| !sensors::is_apple_support_chip(&s.label))
            .filter(|s| {
                let label = s.label.to_ascii_lowercase();
                keys.iter().any(|key| label.contains(key))
            })
            .map(|s| s.value)
            .reduce(f32::max)
    };
    if let Some(max) = hottest_matching(&["package", "tctl", "tdie", "cpu", "soc"])
        .or_else(|| hottest_matching(&["core"]))
    {
        return Some(max);
    }

    mean(
        readings()
            .filter(|s| s.label.starts_with("PMU tdie"))
            .map(|s| s.value)
            .collect(),
    )
}

/// 用传感器读数补上 CPU 温度。
pub fn enrich_with_sensors(cpu: &mut CpuInfo, sensors: &[SensorReading]) {
    if let Some(t) = cpu_temperature(sensors) {
        cpu.temperature_c = Some(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str, value: f32) -> SensorReading {
        SensorReading {
            source: "test".to_string(),
            label: label.to_string(),
            kind: "temperature".to_string(),
            value,
            unit: "C".to_string(),
        }
    }

    #[test]
    fn apple_silicon_averages_the_core_sensors_and_ignores_the_pmu() {
        // 满载的 M2 Max：PMU 才 57 度，核心都在八十几度，用户在别的软件里看到的是后者。
        let sensors = vec![
            temp("PMU tdie0", 56.6),
            temp("PMU tdie1", 57.2),
            temp("NAND CH0 temp", 39.0),
            temp("gas gauge battery", 29.8),
            temp("CPU efficiency core 1", 80.0),
            temp("CPU efficiency core 2", 84.0),
            temp("CPU performance core 1", 90.0),
            temp("CPU performance core 2", 94.0),
        ];
        assert_eq!(cpu_temperature(&sensors), Some(87.0));
    }

    #[test]
    fn hid_core_sensors_count_but_the_rest_of_the_soc_does_not() {
        let sensors = vec![
            temp("pACC MTR Temp Sensor0", 70.0),
            temp("pACC MTR Temp Sensor1", 74.0),
            temp("eACC MTR Temp Sensor0", 66.0),
            temp("SOC MTR Temp Sensor0", 99.0),
            temp("GPU MTR Temp Sensor0", 98.0),
        ];
        assert_eq!(cpu_temperature(&sensors), Some(70.0));
    }

    #[test]
    fn intel_linux_uses_the_package_temperature() {
        let sensors = vec![
            temp("coretemp Core 0", 66.0),
            temp("coretemp Package id 0", 70.0),
            temp("coretemp Core 1", 68.0),
            temp("nvme Composite", 41.0),
            temp("acpitz temp1", 30.0),
        ];
        assert_eq!(cpu_temperature(&sensors), Some(70.0));
    }

    #[test]
    fn amd_uses_tctl_rather_than_the_ccd_sensors() {
        let sensors = vec![
            temp("k10temp Tccd1", 55.0),
            temp("k10temp Tctl", 61.0),
            temp("nvme Composite", 38.0),
        ];
        assert_eq!(cpu_temperature(&sensors), Some(61.0));
    }

    #[test]
    fn falls_back_to_the_hottest_core_without_a_package_sensor() {
        let sensors = vec![temp("coretemp Core 0", 60.0), temp("coretemp Core 1", 64.0)];
        assert_eq!(cpu_temperature(&sensors), Some(64.0));
    }

    #[test]
    fn intel_mac_uses_the_cpu_proximity_sensor() {
        let sensors = vec![
            temp("CPU Proximity", 55.0),
            temp("GPU", 60.0),
            temp("Battery", 30.0),
        ];
        assert_eq!(cpu_temperature(&sensors), Some(55.0));
    }

    #[test]
    fn the_pmu_die_is_only_a_last_resort() {
        let sensors = vec![
            temp("PMU tdie0", 56.0),
            temp("PMU tdie1", 58.0),
            temp("PMU tdev1", 40.0),
            temp("SOC MTR Temp Sensor0", 70.0),
            temp("NAND CH0 temp", 39.0),
        ];
        // 只有 tdie 才算，tdev 是外设侧的温度，SOC MTR 是 SoC 里别的模块。
        assert_eq!(cpu_temperature(&sensors), Some(57.0));
    }

    #[test]
    fn nothing_that_is_not_a_cpu_sensor_is_taken_for_one() {
        let sensors = vec![
            temp("SOC MTR Temp Sensor0", 70.0),
            temp("NAND CH0 temp", 39.0),
            temp("gas gauge battery", 29.8),
            temp("nvme Composite", 41.0),
            temp("GPU 1", 75.0),
        ];
        assert_eq!(cpu_temperature(&sensors), None);
        assert_eq!(cpu_temperature(&[]), None);
    }

    #[test]
    fn ignores_other_kinds_and_broken_values() {
        let mut fan = temp("CPU fan", 3000.0);
        fan.kind = "fan".to_string();
        let sensors = vec![fan, temp("CPU Proximity", f32::NAN), temp("CPU Die", 52.0)];
        assert_eq!(cpu_temperature(&sensors), Some(52.0));
    }

    #[test]
    fn enriching_the_snapshot_sets_the_cpu_temperature() {
        let mut info = CpuInfo::default();
        enrich_with_sensors(&mut info, &[temp("CPU performance core 1", 80.0)]);
        assert_eq!(info.temperature_c, Some(80.0));
        // 读不到的时候保留原值，不要被清掉。
        enrich_with_sensors(&mut info, &[]);
        assert_eq!(info.temperature_c, Some(80.0));
    }
}
