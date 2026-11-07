//! 大屏的数据：独立的走势历史，以及各组件「挑最有代表性的那一项」的小函数。
//!
//! 走势单独记，不和「实时监控」页共用：那一页可以暂停、清空、改保留时长，都不该影响大屏。

use super::config::WidgetKind;
use crate::menubar::data::system_volume;
use crate::state::AppState;
use pc_specs_core::model::{GpuInfo, GpuTick, MonitorTick, NetworkInterface, StorageInfo};
use std::collections::VecDeque;

/// 最长保留多久的走势：设置里最长的时间窗口是 10 分钟，多留几秒免得图表左端被截掉。
const KEEP_MS: u64 = 10 * 60_000 + 5_000;
/// 重采样后连续缺这么多格以内的空洞会被补上（采样偶尔晚一拍很常见），再长就是真的没数据。
const FILL_GAP: usize = 2;

/// 走势里的一个采样。
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub at: u64,
    pub cpu: f32,
    pub mem: f32,
    pub gpu: Option<f32>,
    pub temp: Option<f32>,
    pub down: f32,
    pub up: f32,
    pub read: f32,
    pub write: f32,
}

impl Point {
    fn new(tick: &MonitorTick) -> Self {
        Self {
            at: tick.timestamp,
            cpu: tick.cpu_overall,
            mem: crate::fmt::ratio(tick.mem_used_bytes, tick.mem_total_bytes),
            gpu: tick
                .gpus
                .iter()
                .filter_map(|gpu| gpu.utilization)
                .max_by(f32::total_cmp),
            temp: headline_temperature(tick),
            down: tick.net_rx_bps as f32,
            up: tick.net_tx_bps as f32,
            read: tick.disk_read_bps as f32,
            write: tick.disk_write_bps as f32,
        }
    }
}

/// 温度组件的主读数：优先 CPU，没有明确属于 CPU 的传感器就取全机最高。
pub fn headline_temperature(tick: &MonitorTick) -> Option<f32> {
    tick.cpu_temperature().or_else(|| tick.max_temperature())
}

#[derive(Default)]
pub struct History {
    points: VecDeque<Point>,
}

impl History {
    pub fn push(&mut self, tick: &MonitorTick) {
        // 系统时钟被往回调了：旧的点会让图表错乱，丢掉重来。
        if self
            .points
            .back()
            .is_some_and(|last| tick.timestamp < last.at)
        {
            self.points.clear();
        }
        self.points.push_back(Point::new(tick));
        let oldest = tick.timestamp.saturating_sub(KEEP_MS);
        while self.points.front().is_some_and(|point| point.at < oldest) {
            self.points.pop_front();
        }
    }

    /// 把最近 `window_ms` 毫秒切成 `bins` 个等宽的时间段，每段取平均；没有数据的段是 `None`。
    /// 右端是最新的一个采样，所以图表是随采样一格一格往左推的。
    pub fn bins(
        &self,
        window_ms: u64,
        bins: usize,
        pick: impl Fn(&Point) -> Option<f32>,
    ) -> Vec<Option<f32>> {
        let bins = bins.max(1);
        let Some(end) = self.points.back().map(|point| point.at) else {
            return vec![None; bins];
        };
        // 用有符号数算：历史比窗口短的时候，窗口的起点在第一个采样之前，不能被截成 0。
        let window = window_ms.max(1) as i64;
        let start = end as i64 - window;
        let mut sums = vec![(0.0_f32, 0_u32); bins];
        for point in self
            .points
            .iter()
            .rev()
            .take_while(|p| p.at as i64 >= start)
        {
            let Some(value) = pick(point).filter(|value| value.is_finite()) else {
                continue;
            };
            let slot = ((point.at as i64 - start) * bins as i64 / window).clamp(0, bins as i64 - 1)
                as usize;
            sums[slot].0 += value;
            sums[slot].1 += 1;
        }
        let mut values: Vec<Option<f32>> = sums
            .into_iter()
            .map(|(sum, count)| (count > 0).then(|| sum / count as f32))
            .collect();
        fill_gaps(&mut values);
        values
    }
}

/// 两头都有数据的短空洞线性补齐。
fn fill_gaps(values: &mut [Option<f32>]) {
    let mut index = 0;
    while index < values.len() {
        if values[index].is_some() {
            index += 1;
            continue;
        }
        let run_end = values[index..]
            .iter()
            .position(Option::is_some)
            .map_or(values.len(), |offset| index + offset);
        let before = index.checked_sub(1).and_then(|i| values[i]);
        let after = values.get(run_end).copied().flatten();
        if let (Some(before), Some(after)) = (before, after)
            && run_end - index <= FILL_GAP
        {
            let steps = (run_end - index + 1) as f32;
            for (offset, slot) in values[index..run_end].iter_mut().enumerate() {
                let fraction = (offset + 1) as f32 / steps;
                *slot = Some(before + (after - before) * fraction);
            }
        }
        index = run_end;
    }
}

/// GPU 组件展示的那一块：当前占用最高的；都没有读数时取第一块独显，再取第一块。
pub fn active_gpu(app: &AppState) -> Option<&GpuInfo> {
    let gpus = &app.snapshot.gpus;
    let load = |gpu: &GpuInfo| tick_of(app, &gpu.name).and_then(|tick| tick.utilization);
    gpus.iter()
        .filter(|gpu| load(gpu).is_some())
        .max_by(|a, b| load(a).unwrap_or(0.).total_cmp(&load(b).unwrap_or(0.)))
        .or_else(|| gpus.iter().find(|gpu| gpu.is_discrete))
        .or_else(|| gpus.first())
}

/// 这块 GPU 在最新一次采样里的读数，按名字对应（监控页也是这样对应的）。
fn tick_of<'a>(app: &'a AppState, name: &str) -> Option<&'a GpuTick> {
    app.latest
        .as_ref()?
        .gpus
        .iter()
        .find(|tick| tick.name == name)
}

/// 展示这块 GPU 用的实时读数。快照里的占用率只在启动时读过一次，会一直停在那个数上，
/// 所以大屏上的占用和显存都以采样为准；对不上名字就退而取采样里的第一块。
pub fn live_gpu<'a>(app: &'a AppState, card: Option<&GpuInfo>) -> Option<&'a GpuTick> {
    card.and_then(|card| tick_of(app, &card.name))
        .or_else(|| app.latest.as_ref().and_then(|tick| tick.gpus.first()))
}

/// 优先选已连接、有 IPv4 的物理接口。
pub fn active_interface(app: &AppState) -> Option<&NetworkInterface> {
    crate::pages::dashboard::primary_interface(app)
}

/// 存储组件要列出的卷：系统卷排最前，其余按容量从大到小。
pub fn volumes(app: &AppState) -> Vec<&StorageInfo> {
    let system = system_volume(&app.snapshot);
    let mut volumes: Vec<&StorageInfo> = app
        .snapshot
        .storages
        .iter()
        .filter(|volume| volume.total_bytes > 0)
        .collect();
    volumes.sort_by_key(|volume| {
        (
            !system.is_some_and(|system| std::ptr::eq(system, *volume)),
            std::cmp::Reverse(volume.total_bytes),
        )
    });
    volumes
}

/// 这块硬件在这台机器上有没有读数。没有的组件，在「隐藏缺失硬件」打开时不占位。
pub fn available(kind: WidgetKind, app: &AppState) -> bool {
    match kind {
        WidgetKind::Gpu => {
            !app.snapshot.gpus.is_empty() || app.latest.as_ref().is_some_and(|t| !t.gpus.is_empty())
        }
        WidgetKind::Temperature => app
            .latest
            .as_ref()
            .is_some_and(|tick| headline_temperature(tick).is_some()),
        WidgetKind::Battery => app.snapshot.battery.is_some(),
        WidgetKind::Storage => app.snapshot.storages.iter().any(|v| v.total_bytes > 0),
        WidgetKind::Clock
        | WidgetKind::Cpu
        | WidgetKind::Memory
        | WidgetKind::Network
        | WidgetKind::DiskIo
        | WidgetKind::Processes
        | WidgetKind::System => true,
    }
}
