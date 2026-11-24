//! 按实际时间保留监控历史，采样间隔变化时不丢失仍在范围内的数据。

use gpui_kit::SharedString;
use pc_specs_core::model::{GpuTick, MonitorTick};
use std::collections::VecDeque;
use std::time::Duration;

// 最长 10 分钟、最快 250ms 一帧，另留一个端点。时钟回拨时也限制内存占用。
const MAX_SAMPLES: usize = 2401;

/// Charts retain scalar readings only. Per-core, disk, interface and sensor
/// inventories belong to the current frame and can be large on a busy host.
#[derive(Default)]
pub struct Sample {
    pub timestamp: u64,
    pub label: SharedString,
    pub cpu_overall: f32,
    pub memory_percent: f32,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub max_temperature: Option<f32>,
    pub cpu_temperature: Option<f32>,
    pub gpus: Vec<GpuTick>,
}

impl From<&MonitorTick> for Sample {
    fn from(tick: &MonitorTick) -> Self {
        Self {
            timestamp: tick.timestamp,
            label: crate::fmt::clock(tick.timestamp).into(),
            cpu_overall: tick.cpu_overall,
            memory_percent: crate::fmt::ratio(tick.mem_used_bytes, tick.mem_total_bytes),
            net_rx_bps: tick.net_rx_bps,
            net_tx_bps: tick.net_tx_bps,
            disk_read_bps: tick.disk_read_bps,
            disk_write_bps: tick.disk_write_bps,
            max_temperature: tick.max_temperature(),
            cpu_temperature: tick.cpu_temperature(),
            gpus: tick.gpus.clone(),
        }
    }
}

pub fn trim(ticks: &mut VecDeque<Sample>, range: Duration) {
    let Some(newest) = ticks.back() else { return };
    let cutoff = newest.timestamp.saturating_sub(range.as_millis() as u64);
    while ticks.len() > MAX_SAMPLES || ticks.front().is_some_and(|t| t.timestamp < cutoff) {
        ticks.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(times: impl IntoIterator<Item = u64>) -> VecDeque<Sample> {
        times
            .into_iter()
            .map(|timestamp| Sample {
                timestamp,
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn chart_history_preserves_the_displayed_readings() {
        use pc_specs_core::model::SensorReading;
        let tick = MonitorTick {
            timestamp: 5000,
            mem_total_bytes: 100,
            mem_used_bytes: 75,
            net_tx_bps: 16,
            temperatures: vec![SensorReading {
                label: "CPU package".into(),
                kind: "temperature".into(),
                value: 65.,
                ..Default::default()
            }],
            ..Default::default()
        };
        let point = Sample::from(&tick);
        assert_eq!(point.timestamp, tick.timestamp);
        assert_eq!(point.memory_percent, 75.);
        assert_eq!(point.net_tx_bps, 16);
        assert_eq!(point.cpu_temperature, Some(65.));
        assert_eq!(point.max_temperature, Some(65.));
    }

    #[test]
    fn keeps_full_ten_minutes_at_fastest_interval() {
        let mut ticks = history((0..=2400).map(|n| n * 250));
        trim(&mut ticks, Duration::from_secs(600));
        assert_eq!(ticks.len(), 2401);
        assert_eq!(ticks.front().unwrap().timestamp, 0);
    }

    #[test]
    fn mixed_intervals_are_trimmed_by_age() {
        let mut ticks = history([0, 10_000, 59_000, 60_000, 60_250, 70_000]);
        trim(&mut ticks, Duration::from_secs(60));
        assert_eq!(ticks.front().unwrap().timestamp, 10_000);
        assert_eq!(
            ticks.iter().map(|t| t.timestamp).collect::<Vec<_>>(),
            [10_000, 59_000, 60_000, 60_250, 70_000]
        );
        trim(&mut ticks, Duration::from_secs(10));
        assert_eq!(
            ticks.iter().map(|t| t.timestamp).collect::<Vec<_>>(),
            [60_000, 60_250, 70_000]
        );
    }

    #[test]
    fn bounds_memory_when_clock_moves_backwards() {
        let mut ticks = history((0..5000).rev());
        trim(&mut ticks, Duration::from_secs(600));
        assert_eq!(ticks.len(), MAX_SAMPLES);
    }
}
