use crate::model::SensorReading;
use crate::state::SharedSys;
use std::collections::HashSet;
use std::sync::Arc;
use sysinfo::Components;

pub fn collect(_shared: &Arc<SharedSys>) -> Vec<SensorReading> {
    let mut out: Vec<SensorReading> = Vec::new();
    let components = Components::new_with_refreshed_list();
    for c in components.iter() {
        let temp = c.temperature();
        if !temp.is_nan() && temp > -50.0 && temp < 200.0 {
            out.push(SensorReading {
                source: "sysinfo".to_string(),
                label: c.label().to_string(),
                kind: "temperature".to_string(),
                value: temp,
                unit: "C".to_string(),
            });
        }
    }
    out.extend(crate::platform::extra_sensors());

    // 去重：(kind, label_lower) 作为 key；保留首次出现的（sysinfo 优先于 platform extras）。
    let mut seen: HashSet<(String, String)> = HashSet::new();
    out.retain(|s| seen.insert((s.kind.clone(), s.label.to_lowercase())));
    out
}
