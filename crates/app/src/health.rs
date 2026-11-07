//! 健康度评估：把实时采样与磁盘占用折算成「正常 / 警告 / 严重」。
//!
//! 阈值沿用旧版：CPU ≥95 严重；内存与磁盘 ≥95 严重、85–95 警告；最高温度 ≥88 严重、75–88 警告。

use pc_specs_core::model::{MonitorTick, StorageInfo};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Healthy,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Cpu,
    Memory,
    Disk,
    Temperature,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub kind: AlertKind,
    pub level: Level,
    /// 百分比（CPU / 内存 / 磁盘）或摄氏度（温度）。
    pub value: f32,
    /// 磁盘告警对应的卷名。
    pub detail: Option<String>,
}

fn classify(value: f32, warn: f32, danger: f32) -> Level {
    if value >= danger {
        Level::Critical
    } else if value >= warn {
        Level::Warning
    } else {
        Level::Healthy
    }
}

/// 颜色分级：占用率条使用同一套阈值。
pub fn usage_level(percent: f32) -> Level {
    classify(percent, 75.0, 90.0)
}

/// 电量越低越危险，和占用率的方向相反。
pub fn battery_level(percent: f32) -> Level {
    if percent < 15. {
        Level::Critical
    } else if percent < 30. {
        Level::Warning
    } else {
        Level::Healthy
    }
}

/// 温度分级：与告警使用同一套阈值。
pub fn temperature_level(celsius: f32) -> Level {
    classify(celsius, 75.0, 88.0)
}

pub fn evaluate(tick: Option<&MonitorTick>, storages: &[StorageInfo]) -> Vec<Alert> {
    let mut alerts = Vec::new();

    if let Some(t) = tick {
        if t.cpu_overall >= 95.0 {
            alerts.push(Alert {
                kind: AlertKind::Cpu,
                level: Level::Critical,
                value: t.cpu_overall,
                detail: None,
            });
        }
        if t.mem_total_bytes > 0 {
            let pct = t.mem_used_bytes as f32 / t.mem_total_bytes as f32 * 100.0;
            let level = classify(pct, 85.0, 95.0);
            if level != Level::Healthy {
                alerts.push(Alert {
                    kind: AlertKind::Memory,
                    level,
                    value: pct,
                    detail: None,
                });
            }
        }
        if let Some(temp) = t.max_temperature() {
            let level = temperature_level(temp);
            if level != Level::Healthy {
                alerts.push(Alert {
                    kind: AlertKind::Temperature,
                    level,
                    value: temp,
                    detail: None,
                });
            }
        }
    }

    // 磁盘只报最满的那一块，避免多卷时刷屏。
    let fullest = storages
        .iter()
        .filter(|s| s.total_bytes > 0)
        .map(|s| (s, s.used_bytes as f32 / s.total_bytes as f32 * 100.0))
        .max_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((disk, pct)) = fullest {
        let level = classify(pct, 85.0, 95.0);
        if level != Level::Healthy {
            alerts.push(Alert {
                kind: AlertKind::Disk,
                level,
                value: pct,
                detail: Some(
                    disk.mount_point
                        .clone()
                        .unwrap_or_else(|| disk.name.clone()),
                ),
            });
        }
    }

    alerts
}

pub fn overall(alerts: &[Alert]) -> Level {
    alerts
        .iter()
        .map(|a| a.level)
        .max()
        .unwrap_or(Level::Healthy)
}
