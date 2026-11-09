//! 健康度评估：把实时采样与磁盘占用折算成「正常 / 警告 / 严重」。
//!
//! 阈值：CPU ≥95 严重；内存与磁盘 ≥95 严重、85–95 警告；温度 ≥100 严重、90–100 警告
//! （Apple 芯片和 Intel 处理器都在 100 度上下开始降频，核心满载时八九十度是正常的）。

use pc_specs_core::model::{MonitorTick, StorageInfo};
use pc_specs_core::netprobe::TargetKind;

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

/// 温度分级的两个阈值（摄氏度），按处理器核心的标准定：满载时八九十度是正常的，
/// 到 [`TEMP_WARNING`] 提醒一下，到 [`TEMP_CRITICAL`] 上下就快要降频了。
pub const TEMP_WARNING: f32 = 90.0;
pub const TEMP_CRITICAL: f32 = 100.0;

/// 温度分级：与告警使用同一套阈值。
pub fn temperature_level(celsius: f32) -> Level {
    classify(celsius, TEMP_WARNING, TEMP_CRITICAL)
}

/// 热压力等级（0 正常、1 中等、2 较重、3 严重、4 休眠）分级：中等只是系统开始留意温度，
/// 较重就已经在压低性能了。
pub fn thermal_level(pressure: u8) -> Level {
    match pressure {
        0 | 1 => Level::Healthy,
        2 => Level::Warning,
        _ => Level::Critical,
    }
}

/// 内存告警的等级：系统自己给了内存压力（macOS、Linux 的 PSI）就以它为准——macOS 上内存用到九成
/// 是常态，压力才说明有没有问题；没有压力读数的平台才看占用率。
pub fn memory_level(used_percent: f32, pressure: Option<u8>) -> Level {
    match pressure {
        Some(0) => Level::Healthy,
        Some(1) => Level::Warning,
        Some(_) => Level::Critical,
        None => classify(used_percent, 85.0, 95.0),
    }
}

/// 磁盘告警的等级：占用率高、剩下的空间也真的不多才算。4 TB 的盘用到 95% 还剩 200 GB，
/// 不用报警；256 GB 的盘用到 90% 只剩 25 GB，已经很紧了。
pub fn disk_level(used_percent: f32, free_bytes: u64) -> Level {
    const GIB: u64 = 1 << 30;
    if used_percent >= 90.0 && free_bytes < 30 * GIB {
        Level::Critical
    } else if used_percent >= 85.0 && free_bytes < 100 * GIB {
        Level::Warning
    } else {
        Level::Healthy
    }
}

/// 延迟分级。局域网这一跳（路由器）的标准比公网严得多。
pub fn latency_level(kind: TargetKind, ms: f32) -> Level {
    match kind {
        TargetKind::Gateway => classify(ms, 15.0, 50.0),
        TargetKind::Dns | TargetKind::Internet => classify(ms, 60.0, 150.0),
    }
}

/// Wi-Fi 信号质量（0–100）分级，和占用率的方向相反：越弱越差。
pub fn signal_level(quality: u8) -> Level {
    if quality < 25 {
        Level::Critical
    } else if quality < 50 {
        Level::Warning
    } else {
        Level::Healthy
    }
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
            let level = memory_level(pct, t.memory.as_ref().and_then(|m| m.pressure_level));
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

    // 磁盘只报最紧的那一块，避免多卷时刷屏：先比等级，同级的比谁更满。
    let tightest = storages
        .iter()
        .filter(|s| s.total_bytes > 0)
        .map(|s| {
            let pct = s.used_bytes as f32 / s.total_bytes as f32 * 100.0;
            let free = s.total_bytes.saturating_sub(s.used_bytes);
            (s, pct, disk_level(pct, free))
        })
        .max_by(|a, b| a.2.cmp(&b.2).then(a.1.total_cmp(&b.1)));
    if let Some((disk, pct, level)) = tightest
        && level != Level::Healthy
    {
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

    alerts
}

pub fn overall(alerts: &[Alert]) -> Level {
    alerts
        .iter()
        .map(|a| a.level)
        .max()
        .unwrap_or(Level::Healthy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn volume(total_gib: u64, free_gib: u64) -> StorageInfo {
        StorageInfo {
            name: "disk".to_string(),
            mount_point: Some(format!("/vol{total_gib}")),
            total_bytes: total_gib * GIB,
            used_bytes: (total_gib - free_gib) * GIB,
            ..Default::default()
        }
    }

    #[test]
    fn a_big_disk_with_plenty_left_is_not_an_alarm() {
        // 4 TB 的盘用到 95%，还剩 200 GB。
        assert_eq!(disk_level(95.0, 200 * GIB), Level::Healthy);
        assert_eq!(disk_level(99.0, 150 * GIB), Level::Healthy);
    }

    #[test]
    fn a_small_disk_getting_full_is_flagged_by_both_measures() {
        assert_eq!(disk_level(84.0, 10 * GIB), Level::Healthy);
        assert_eq!(disk_level(85.0, 40 * GIB), Level::Warning);
        assert_eq!(disk_level(92.0, 25 * GIB), Level::Critical);
        // 占用率很高但剩下的还有几十 GB：警告，不到严重。
        assert_eq!(disk_level(97.0, 60 * GIB), Level::Warning);
    }

    #[test]
    fn the_alert_names_the_tightest_volume_not_the_fullest_percentage() {
        // 一块大盘 96% 但还剩 160 GB，一块小盘 88% 只剩 14 GB：该报的是小盘。
        let big = volume(4000, 160);
        let small = volume(120, 14);
        let alerts = evaluate(None, &[big, small]);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, AlertKind::Disk);
        assert_eq!(alerts[0].detail.as_deref(), Some("/vol120"));
    }

    #[test]
    fn memory_follows_the_systems_pressure_when_it_reports_one() {
        // macOS 上用到 92% 但压力是绿的：正常。
        assert_eq!(memory_level(92.0, Some(0)), Level::Healthy);
        // 占用率不高但系统已经在报压力：照报。
        assert_eq!(memory_level(60.0, Some(1)), Level::Warning);
        assert_eq!(memory_level(60.0, Some(2)), Level::Critical);
        // 没有压力读数的平台看占用率。
        assert_eq!(memory_level(80.0, None), Level::Healthy);
        assert_eq!(memory_level(90.0, None), Level::Warning);
        assert_eq!(memory_level(96.0, None), Level::Critical);
    }

    #[test]
    fn thermal_pressure_only_alarms_once_performance_is_cut() {
        assert_eq!(thermal_level(0), Level::Healthy);
        assert_eq!(thermal_level(1), Level::Healthy);
        assert_eq!(thermal_level(2), Level::Warning);
        assert_eq!(thermal_level(3), Level::Critical);
        assert_eq!(thermal_level(4), Level::Critical);
    }

    #[test]
    fn temperatures_use_the_core_scale() {
        assert_eq!(temperature_level(88.0), Level::Healthy);
        assert_eq!(temperature_level(90.0), Level::Warning);
        assert_eq!(temperature_level(99.9), Level::Warning);
        assert_eq!(temperature_level(100.0), Level::Critical);
    }
}
