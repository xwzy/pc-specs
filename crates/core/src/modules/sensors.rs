use crate::model::SensorReading;
use std::collections::HashSet;
use sysinfo::Components;

/// 全部传感器：sysinfo 的温度 + 各平台独有的风扇 / 电压 / 功率 / 热区。
pub fn collect() -> Vec<SensorReading> {
    let mut out = match Components::new_with_refreshed_list() {
        Ok(c) => temperatures(&c),
        Err(_) => Vec::new(),
    };
    out.extend(crate::platform::extra_sensors());

    // 去重：(kind, label_lower)；保留先出现的（sysinfo 优先于平台补充）。
    let mut seen: HashSet<(String, String)> = HashSet::new();
    out.retain(|s| seen.insert((s.kind.clone(), s.label.to_lowercase())));
    out
}

/// Apple 芯片的 CPU 逐核温度传感器：SMC 键表读出来的（`CPU performance core 3`），
/// 以及部分芯片的 HID 里直接给的 `pACC MTR Temp Sensor0` / `eACC MTR Temp Sensor0`。
pub fn is_cpu_core(label: &str) -> bool {
    (label.starts_with("CPU ") && label.contains(" core "))
        || label.starts_with("pACC MTR")
        || label.starts_with("eACC MTR")
}

/// GPU 温度传感器：Apple 芯片 SMC 键表读出来的（`GPU 2`）、HID 里的 `GPU MTR Temp Sensor0`，
/// 以及 Intel Mac 上 sysinfo 给的 `GPU`。
pub fn is_gpu_core(label: &str) -> bool {
    label == "GPU" || label.starts_with("GPU ")
}

/// Apple 芯片 HID 里的电源管理芯片、SoC 其余模块的传感器。名字里带 `tdie` / `soc`，
/// 但它们说的不是 CPU 核心：PMU 是另一颗芯片，满载时比 CPU 核心低三十多度。
pub(crate) fn is_apple_support_chip(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    lower.starts_with("pmu") || lower.starts_with("pmgr") || lower.contains(" mtr ")
}

/// 从已刷新的 `Components` 取温度。监控线程复用同一个 `Components`，避免每次重新枚举。
pub(crate) fn temperatures(components: &Components) -> Vec<SensorReading> {
    components
        .iter()
        .filter_map(|c| {
            let temp = c.temperature()?;
            // 过滤掉掉线传感器常见的哨兵值（0 以下 / 超高）。
            (temp.is_finite() && temp > -50.0 && temp < 200.0).then(|| SensorReading {
                source: "sysinfo".to_string(),
                label: c.label().to_string(),
                kind: "temperature".to_string(),
                value: temp,
                unit: "C".to_string(),
            })
        })
        .collect()
}
