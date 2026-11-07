//! 监控大屏的设置：放在哪块屏幕、用什么主题、哪些组件按什么顺序和大小排列。
//!
//! 所有字段都有默认值（`#[serde(default)]`），旧版设置文件读进来不会出错；之后新增字段只增不删。

use super::theme::ThemeId;
use crate::settings::WindowState;
use gpui_kit::assets::IconName;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 大屏上可以摆放的组件。每种只有一个，开关、顺序和大小都在 `BigScreenSettings::widgets` 里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Clock,
    Cpu,
    Memory,
    Gpu,
    Network,
    DiskIo,
    Storage,
    Temperature,
    Battery,
    Processes,
    System,
}

impl WidgetKind {
    pub const ALL: [Self; 11] = [
        Self::Cpu,
        Self::Memory,
        Self::Gpu,
        Self::Network,
        Self::DiskIo,
        Self::Temperature,
        Self::Storage,
        Self::Battery,
        Self::Processes,
        Self::System,
        Self::Clock,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Gpu => "gpu",
            Self::Network => "network",
            Self::DiskIo => "disk_io",
            Self::Storage => "storage",
            Self::Temperature => "temperature",
            Self::Battery => "battery",
            Self::Processes => "processes",
            Self::System => "system",
        }
    }

    pub fn title(self) -> String {
        let text = match self {
            Self::Clock => t!("bs_w_clock"),
            Self::Cpu => t!("nav_cpu"),
            Self::Memory => t!("nav_memory"),
            Self::Gpu => t!("nav_gpu"),
            Self::Network => t!("nav_network"),
            Self::DiskIo => t!("bs_w_disk_io"),
            Self::Storage => t!("nav_storage"),
            Self::Temperature => t!("bs_w_temperature"),
            Self::Battery => t!("nav_battery"),
            Self::Processes => t!("nav_processes"),
            Self::System => t!("bs_w_system"),
        };
        text.into_owned()
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Clock => IconName::Clock,
            Self::Cpu => IconName::Cpu,
            Self::Memory => IconName::MemoryStick,
            Self::Gpu => IconName::Gpu,
            Self::Network => IconName::Network,
            Self::DiskIo => IconName::ArrowLeftRight,
            Self::Storage => IconName::HardDrive,
            Self::Temperature => IconName::Thermometer,
            Self::Battery => IconName::BatteryCharging,
            Self::Processes => IconName::List,
            Self::System => IconName::Laptop,
        }
    }

    /// 新增组件（旧设置文件里没有它）时用的大小。
    fn default_cell(self) -> (u8, u8) {
        match self {
            Self::Network => (2, 1),
            Self::Processes => (1, 2),
            _ => (1, 1),
        }
    }
}

/// 一个组件的摆放：开关，以及相对宽高（1–3）。磁贴的面积与「宽×高」成正比。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetConfig {
    pub kind: WidgetKind,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "one")]
    pub w: u8,
    #[serde(default = "one")]
    pub h: u8,
}

fn yes() -> bool {
    true
}

fn one() -> u8 {
    1
}

pub const CELL_RANGE: std::ops::RangeInclusive<u8> = 1..=3;
pub const SCALE_RANGE: std::ops::RangeInclusive<u32> = 70..=150;
pub const SCALE_STEP: u32 = 10;

/// 图表样式。`Auto` 跟随主题（终端风格的主题默认用格栅，其余用面积图）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartPref {
    #[default]
    Auto,
    Area,
    Line,
    Bars,
}

/// 磁贴之间的留白。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Normal,
    Relaxed,
}

impl Density {
    /// 设计尺寸（1280×720 的屏幕上）里的间距，实际按屏幕大小等比放大。
    pub fn gap(self) -> f32 {
        match self {
            Self::Compact => 8.,
            Self::Normal => 14.,
            Self::Relaxed => 22.,
        }
    }
}

/// 进程组件按什么排序。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rank {
    #[default]
    Cpu,
    Memory,
}

/// 设置页预览框的比例：用来直接看到各种屏幕形状下的排版。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Landscape,
    Portrait,
    Ultrawide,
    Square,
}

impl Shape {
    pub const ALL: [Self; 4] = [
        Self::Landscape,
        Self::Portrait,
        Self::Ultrawide,
        Self::Square,
    ];

    /// 宽 : 高。
    pub fn ratio(self) -> (f32, f32) {
        match self {
            Self::Landscape => (16., 9.),
            Self::Portrait => (9., 16.),
            Self::Ultrawide => (21., 9.),
            Self::Square => (4., 3.),
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Landscape => t!("bs_shape_landscape"),
            Self::Portrait => t!("bs_shape_portrait"),
            Self::Ultrawide => t!("bs_shape_ultrawide"),
            Self::Square => t!("bs_shape_square"),
        }
        .into_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BigScreenSettings {
    // ---- 显示与窗口 ----
    /// 目标显示器的稳定标识（`PlatformDisplay::uuid`）；`None` 表示自动：有副屏就用副屏。
    pub display: Option<String>,
    pub fullscreen: bool,
    /// 窗口模式下上次的位置与大小。
    pub window: Option<WindowState>,
    pub open_on_launch: bool,
    /// 大屏开着的时候不让屏幕熄灭、不让系统休眠。
    pub keep_awake: bool,

    // ---- 外观 ----
    pub theme: ThemeId,
    /// 内容缩放（百分比）：越大，每个磁贴里的字越大、能放下的细节越少。
    pub scale: u32,
    pub density: Density,
    pub show_header: bool,
    /// 顶栏标题，留空时用主机名。
    pub title: String,
    pub show_clock: bool,
    pub show_seconds: bool,
    pub hour24: bool,
    pub chart: ChartPref,
    /// 趋势图覆盖的时长（分钟）。
    pub chart_minutes: u32,

    // ---- 布局 ----
    /// 机器上没有的硬件（电池、独显、传感器）不占位。
    pub hide_missing: bool,
    pub widgets: Vec<WidgetConfig>,
    /// CPU 组件里显示每个核心的占用。
    pub per_core: bool,
    pub rank: Rank,

    /// 设置页预览框的比例。
    pub preview: Shape,
}

impl Default for BigScreenSettings {
    fn default() -> Self {
        Self {
            display: None,
            fullscreen: true,
            window: None,
            open_on_launch: false,
            keep_awake: true,
            theme: ThemeId::default(),
            scale: 100,
            density: Density::Normal,
            show_header: true,
            title: String::new(),
            show_clock: true,
            show_seconds: true,
            hour24: true,
            chart: ChartPref::Auto,
            chart_minutes: 5,
            hide_missing: true,
            widgets: Preset::Overview.widgets(),
            per_core: true,
            rank: Rank::Cpu,
            preview: Shape::Landscape,
        }
    }
}

impl BigScreenSettings {
    /// 把手工改坏或旧版本缺失的值收敛回合法范围：每种组件恰好一个，尺寸、缩放在范围内。
    pub fn normalize(&mut self) {
        let mut seen = HashSet::new();
        self.widgets.retain(|widget| seen.insert(widget.kind));
        for kind in WidgetKind::ALL {
            if seen.insert(kind) {
                let (w, h) = kind.default_cell();
                self.widgets.push(WidgetConfig {
                    kind,
                    enabled: false,
                    w,
                    h,
                });
            }
        }
        for widget in &mut self.widgets {
            widget.w = widget.w.clamp(*CELL_RANGE.start(), *CELL_RANGE.end());
            widget.h = widget.h.clamp(*CELL_RANGE.start(), *CELL_RANGE.end());
        }
        self.scale = self.scale.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
        if !matches!(self.chart_minutes, 1 | 2 | 5 | 10) {
            self.chart_minutes = 5;
        }
        self.title = self.title.chars().take(48).collect();
        if self.display.as_ref().is_some_and(|id| id.trim().is_empty()) {
            self.display = None;
        }
    }

    pub fn widget(&self, kind: WidgetKind) -> Option<&WidgetConfig> {
        self.widgets.iter().find(|widget| widget.kind == kind)
    }

    /// 这个组件现在是不是开着。
    pub fn uses(&self, kind: WidgetKind) -> bool {
        self.widget(kind).is_some_and(|widget| widget.enabled)
    }

    pub fn move_widget(&mut self, kind: WidgetKind, up: bool) {
        let Some(index) = self.widgets.iter().position(|w| w.kind == kind) else {
            return;
        };
        let target = if up {
            index.checked_sub(1)
        } else {
            index.checked_add(1).filter(|&i| i < self.widgets.len())
        };
        if let Some(target) = target {
            self.widgets.swap(index, target);
        }
    }

    pub fn edit_widget(&mut self, kind: WidgetKind, change: impl FnOnce(&mut WidgetConfig)) {
        if let Some(widget) = self.widgets.iter_mut().find(|w| w.kind == kind) {
            change(widget);
        }
    }

    /// 趋势图的时间窗口（毫秒）。
    pub fn chart_window_ms(&self) -> u64 {
        u64::from(self.chart_minutes) * 60_000
    }
}

/// 一键布局方案：只改组件的开关、顺序和大小，主题、屏幕这些保持不动。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Overview,
    Performance,
    Network,
    Minimal,
    Developer,
}

impl Preset {
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Performance,
        Self::Network,
        Self::Minimal,
        Self::Developer,
    ];

    pub fn title(self) -> String {
        match self {
            Self::Overview => t!("bs_preset_overview"),
            Self::Performance => t!("bs_preset_performance"),
            Self::Network => t!("bs_preset_network"),
            Self::Minimal => t!("bs_preset_minimal"),
            Self::Developer => t!("bs_preset_developer"),
        }
        .into_owned()
    }

    /// 开着的组件，按显示顺序，带相对宽高。
    fn layout(self) -> &'static [(WidgetKind, u8, u8)] {
        use WidgetKind::*;
        match self {
            Self::Overview => &[
                (Cpu, 2, 1),
                (Memory, 1, 1),
                (Gpu, 1, 1),
                (Network, 2, 1),
                (DiskIo, 1, 1),
                (Temperature, 1, 1),
                (Storage, 1, 1),
                (Processes, 1, 2),
                (Battery, 1, 1),
            ],
            Self::Performance => &[
                (Cpu, 2, 2),
                (Memory, 1, 1),
                (Gpu, 1, 1),
                (Temperature, 1, 1),
                (Processes, 1, 2),
            ],
            Self::Network => &[
                (Network, 3, 2),
                (DiskIo, 2, 1),
                (Storage, 1, 1),
                (Cpu, 1, 1),
                (Memory, 1, 1),
            ],
            Self::Minimal => &[
                (Cpu, 1, 1),
                (Memory, 1, 1),
                (Gpu, 1, 1),
                (Network, 1, 1),
                (Temperature, 1, 1),
            ],
            Self::Developer => &[
                (Cpu, 2, 1),
                (Memory, 1, 1),
                (Processes, 2, 2),
                (Network, 1, 1),
                (DiskIo, 1, 1),
                (Storage, 1, 1),
                (System, 1, 1),
            ],
        }
    }

    /// 方案对应的完整组件列表：方案里的在前，其余的关着排在后面。
    pub fn widgets(self) -> Vec<WidgetConfig> {
        let layout = self.layout();
        let mut widgets: Vec<WidgetConfig> = layout
            .iter()
            .map(|&(kind, w, h)| WidgetConfig {
                kind,
                enabled: true,
                w,
                h,
            })
            .collect();
        for kind in WidgetKind::ALL {
            if !layout.iter().any(|(k, ..)| *k == kind) {
                let (w, h) = kind.default_cell();
                widgets.push(WidgetConfig {
                    kind,
                    enabled: false,
                    w,
                    h,
                });
            }
        }
        widgets
    }

    pub fn apply(self, config: &mut BigScreenSettings) {
        config.widgets = self.widgets();
    }
}
