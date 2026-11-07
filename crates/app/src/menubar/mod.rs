pub mod archive;
pub mod config;
mod content;
pub mod data;
#[cfg(target_os = "macos")]
pub mod native;
pub mod panel;
pub mod preferences;
#[cfg(any(target_os = "macos", test))]
pub mod status_image;
pub(crate) mod widgets;

use crate::state::{AppState, Page};
use config::Metric;
use gpui_kit::App;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;

/// 被点击的菜单栏项所在的位置，单位 pt，原点是它所在显示器的左上角（和 GPUI 的窗口坐标一致）。
/// 菜单栏会复制到每块显示器上，弹窗必须跟着用户点的那一块走。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// 显示器的 `CGDirectDisplayID`，GPUI 的 `DisplayId` 用的就是它。
    pub display: u64,
    pub left: f32,
    pub right: f32,
    /// 菜单栏下沿到显示器顶部的距离。
    pub bottom: f32,
}

/// 最近 `last` 个采样，旧的在前。
fn recent(s: &AppState, last: usize) -> impl Iterator<Item = &data::Sample> {
    s.menu_history
        .iter()
        .skip(s.menu_history.len().saturating_sub(last))
}

/// 面板里走势图的柱子数：默认每秒一个采样时，「实时」正好是一分钟。
pub const COLUMNS: usize = 60;

/// 面板里一张走势图的数据。「实时」是最近 `COLUMNS` 个采样（一个采样一根柱子），
/// 其余档位从长期历史里按时间均分成 `COLUMNS` 根；`None` 表示那一段没有读数。
pub fn series(s: &AppState, range: config::TimeRange, channel: data::Channel) -> archive::Series {
    if range != config::TimeRange::Live {
        return s.archive.series(range, channel, archive::now_ms(), COLUMNS);
    }
    let columns: Vec<Option<f64>> = recent(s, COLUMNS)
        .map(|sample| sample.value(channel))
        .collect();
    let known = || columns.iter().flatten().copied();
    let count = known().count();
    archive::Series {
        mean: (count > 0).then(|| known().sum::<f64>() / count as f64),
        peak: known().reduce(f64::max),
        columns,
    }
}

pub fn color(metric: Metric, cx: &App) -> gpui_kit::Hsla {
    let theme = cx.theme();
    match metric {
        Metric::Cpu => theme.blue,
        Metric::Memory | Metric::Battery => theme.green,
        Metric::Storage => gpui_kit::rgb(0xa78bfa).into(),
        Metric::Temperature => theme.yellow,
        Metric::Network => theme.cyan,
        Metric::Gpu => theme.magenta,
    }
}

pub fn icon(metric: Metric) -> IconName {
    match metric {
        Metric::Cpu => IconName::Cpu,
        Metric::Memory => IconName::MemoryStick,
        Metric::Storage => IconName::HardDrive,
        Metric::Temperature => IconName::Thermometer,
        Metric::Network => IconName::Network,
        Metric::Gpu => IconName::Gpu,
        Metric::Battery => IconName::BatteryCharging,
    }
}

pub fn metric_page(metric: Metric) -> Page {
    match metric {
        Metric::Cpu => Page::Cpu,
        Metric::Memory => Page::Memory,
        Metric::Storage => Page::Storage,
        Metric::Temperature => Page::Sensors,
        Metric::Network => Page::Network,
        Metric::Gpu => Page::Gpu,
        Metric::Battery => Page::Battery,
    }
}
