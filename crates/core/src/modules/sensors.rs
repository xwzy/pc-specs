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
