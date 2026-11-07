//! 按实际时间保留监控历史，采样间隔变化时不丢失仍在范围内的数据。

use pc_specs_core::model::MonitorTick;
use std::collections::VecDeque;
use std::time::Duration;

// 最长 10 分钟、最快 250ms 一帧，另留一个端点。时钟回拨时也限制内存占用。
const MAX_SAMPLES: usize = 2401;

pub fn trim<T>(ticks: &mut VecDeque<MonitorTick>, labels: &mut VecDeque<T>, range: Duration) {
    let Some(newest) = ticks.back() else { return };
    let cutoff = newest.timestamp.saturating_sub(range.as_millis() as u64);
    while ticks.len() > MAX_SAMPLES || ticks.front().is_some_and(|t| t.timestamp < cutoff) {
        ticks.pop_front();
        labels.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(times: impl IntoIterator<Item = u64>) -> (VecDeque<MonitorTick>, VecDeque<u64>) {
        let labels: VecDeque<_> = times.into_iter().collect();
        let ticks = labels
            .iter()
            .map(|&timestamp| MonitorTick {
                timestamp,
                ..Default::default()
            })
            .collect();
        (ticks, labels)
    }

    #[test]
    fn keeps_full_ten_minutes_at_fastest_interval() {
        let (mut ticks, mut labels) = history((0..=2400).map(|n| n * 250));
        trim(&mut ticks, &mut labels, Duration::from_secs(600));
        assert_eq!(ticks.len(), 2401);
        assert_eq!(labels.front(), Some(&0));
    }

    #[test]
    fn mixed_intervals_are_trimmed_by_age() {
        let (mut ticks, mut labels) = history([0, 10_000, 59_000, 60_000, 60_250, 70_000]);
        trim(&mut ticks, &mut labels, Duration::from_secs(60));
        assert_eq!(ticks.front().unwrap().timestamp, 10_000);
        assert_eq!(labels, [10_000, 59_000, 60_000, 60_250, 70_000]);
        trim(&mut ticks, &mut labels, Duration::from_secs(10));
        assert_eq!(labels, [60_000, 60_250, 70_000]);
        assert_eq!(ticks.len(), labels.len());
    }

    #[test]
    fn bounds_memory_when_clock_moves_backwards() {
        let (mut ticks, mut labels) = history((0..5000).rev());
        trim(&mut ticks, &mut labels, Duration::from_secs(600));
        assert_eq!(ticks.len(), MAX_SAMPLES);
        assert_eq!(ticks.len(), labels.len());
    }
}
