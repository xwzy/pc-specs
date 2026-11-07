use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Cpu,
    Memory,
    Storage,
    Temperature,
    Network,
    Gpu,
    Battery,
}

impl Metric {
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }
    pub const ALL: [Self; 7] = [
        Self::Cpu,
        Self::Memory,
        Self::Storage,
        Self::Temperature,
        Self::Network,
        Self::Gpu,
        Self::Battery,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Storage => "storage",
            Self::Temperature => "temperature",
            Self::Network => "network",
            Self::Gpu => "gpu",
            Self::Battery => "battery",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "MEM",
            Self::Storage => "DISK",
            Self::Temperature => "TEMP",
            Self::Network => "NET",
            Self::Gpu => "GPU",
            Self::Battery => "BAT",
        }
    }

    pub fn title(self) -> String {
        let key = match self {
            Self::Cpu => "nav_cpu",
            Self::Memory => "nav_memory",
            Self::Storage => "nav_storage",
            Self::Temperature => "nav_sensors",
            Self::Network => "nav_network",
            Self::Gpu => "nav_gpu",
            Self::Battery => "nav_battery",
        };
        rust_i18n::t!(key).into_owned()
    }

    /// 这个指标的代表色（十六进制）：菜单栏位图和托盘图标共用。
    pub fn accent(self) -> &'static str {
        match self {
            Self::Cpu => "#3b82f6",
            Self::Memory | Self::Battery => "#22c55e",
            Self::Network => "#06b6d4",
            Self::Temperature => "#facc15",
            Self::Storage => "#a78bfa",
            Self::Gpu => "#ec4899",
        }
    }

    pub fn needs_processes(self) -> bool {
        matches!(self, Self::Cpu | Self::Memory | Self::Storage)
    }

    /// 这个指标在菜单栏上可以展示的内容，第一项是默认值。
    pub fn shows(self) -> &'static [Show] {
        match self {
            Self::Cpu | Self::Gpu => &[Show::Auto],
            Self::Memory => &[Show::Auto, Show::Used, Show::Free],
            Self::Storage => &[Show::Auto, Show::Free, Show::Used, Show::Io],
            Self::Temperature => &[Show::Auto, Show::Hottest],
            Self::Network => &[Show::Auto, Show::Down, Show::Up],
            Self::Battery => &[Show::Auto, Show::Remaining],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStyle {
    /// 数值在上，标签在下。
    #[default]
    Stacked,
    /// 标签和数值排成一行。旧版本里叫 `compact`。
    #[serde(alias = "compact")]
    Inline,
    /// 左边一小段走势，右边是数值。
    Graph,
    /// 左边一个小环形仪表，右边是数值，和面板里的环形是一个意思。
    Ring,
}

/// 菜单栏上展示指标的哪个读数。`Auto` 是该指标最常看的读数（占用率、上下行……）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Show {
    #[default]
    Auto,
    Used,
    Free,
    Io,
    Hottest,
    Down,
    Up,
    Remaining,
}

impl Show {
    /// 设置页里这个选项的名字；`Auto` 的含义随指标而变。
    pub fn title(self, metric: Metric) -> String {
        let key = match (self, metric) {
            (Self::Auto, Metric::Network) => "bar_show_both",
            (Self::Auto, Metric::Temperature) => "bar_show_cpu_temp",
            (Self::Auto, Metric::Battery) => "bar_show_level",
            (Self::Auto, _) => "bar_show_usage",
            (Self::Used, _) => "bar_show_used",
            (Self::Free, _) => "bar_show_free",
            (Self::Io, _) => "bar_show_io",
            (Self::Hottest, _) => "bar_show_hottest",
            (Self::Down, _) => "bar_download",
            (Self::Up, _) => "bar_upload",
            (Self::Remaining, _) => "bar_show_remaining",
        };
        rust_i18n::t!(key).into_owned()
    }
}

/// 面板里走势图回看多长时间。`Live` 是最近这一两分钟的实时走势，
/// 其余几档来自长期历史（见 `archive`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeRange {
    #[default]
    Live,
    Hour,
    Day,
    Week,
    Month,
}

impl TimeRange {
    pub const ALL: [Self; 5] = [Self::Live, Self::Hour, Self::Day, Self::Week, Self::Month];

    /// 回看的时长（毫秒）。`Live` 取决于采样间隔，没有固定值。
    pub fn span_ms(self) -> Option<u64> {
        const HOUR: u64 = 3_600_000;
        match self {
            Self::Live => None,
            Self::Hour => Some(HOUR),
            Self::Day => Some(24 * HOUR),
            Self::Week => Some(7 * 24 * HOUR),
            Self::Month => Some(30 * 24 * HOUR),
        }
    }

    /// 切换按钮上的字。几档时长的写法各语言通用，只有「实时」要翻译。
    pub fn label(self) -> String {
        match self {
            Self::Live => rust_i18n::t!("range_live").into_owned(),
            Self::Hour => "1h".into(),
            Self::Day => "24h".into(),
            Self::Week => "7d".into(),
            Self::Month => "30d".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuBarItem {
    pub metric: Metric,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub style: ItemStyle,
    #[serde(default)]
    pub show: Show,
}

fn enabled() -> bool {
    true
}

/// 相邻两个监控项之间留白的可调范围，单位 pt。
pub const SPACING_RANGE: std::ops::RangeInclusive<u32> = 0..=24;
/// 设置页里每按一下 +/- 改变的量。
pub const SPACING_STEP: u32 = 2;
pub const DEFAULT_SPACING: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuBarSettings {
    pub enabled: bool,
    /// 监控项最左边的应用图标。没有任何监控项时无论如何都会保留，免得托盘整个消失。
    pub show_icon: bool,
    pub show_labels: bool,
    pub show_units: bool,
    pub separators: bool,
    pub colorful: bool,
    /// 相邻两项之间的留白（pt），位图左右各留一半。
    pub spacing: u32,
    pub refresh_ms: u64,
    pub process_rows: usize,
    pub items: Vec<MenuBarItem>,
}

impl Default for MenuBarSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_icon: true,
            show_labels: true,
            show_units: true,
            separators: false,
            colorful: false,
            spacing: DEFAULT_SPACING,
            refresh_ms: 1000,
            process_rows: 8,
            items: Metric::ALL
                .into_iter()
                .map(|metric| MenuBarItem {
                    metric,
                    enabled: !matches!(metric, Metric::Gpu | Metric::Battery),
                    style: ItemStyle::Stacked,
                    show: Show::Auto,
                })
                .collect(),
        }
    }
}

impl MenuBarSettings {
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|item| seen.insert(item.metric));
        for item in Self::default().items {
            if seen.insert(item.metric) {
                self.items.push(item);
            }
        }
        for item in &mut self.items {
            if !item.metric.shows().contains(&item.show) {
                item.show = Show::Auto;
            }
        }
        if !matches!(self.refresh_ms, 1000 | 2000 | 5000) {
            self.refresh_ms = 1000;
        }
        if !matches!(self.process_rows, 5 | 8 | 12) {
            self.process_rows = 8;
        }
        self.spacing = self
            .spacing
            .clamp(*SPACING_RANGE.start(), *SPACING_RANGE.end());
    }

    /// 每个位图左右各留多少空白（pt）。
    pub fn padding(&self) -> f32 {
        self.spacing as f32 / 2.
    }

    /// 主图标要不要画：没有监控项可看时必须留着，它是找回应用的唯一入口。
    pub fn icon_visible(&self) -> bool {
        self.show_icon || !self.enabled || self.visible().next().is_none()
    }

    pub fn visible(&self) -> impl Iterator<Item = &MenuBarItem> {
        self.items.iter().filter(|item| item.enabled)
    }

    /// 菜单栏 / 托盘上实际展示的监控项：总开关关着时一个都没有。
    pub fn shown(&self) -> impl Iterator<Item = &MenuBarItem> {
        self.items
            .iter()
            .filter(move |item| self.enabled && item.enabled)
    }

    #[cfg(target_os = "macos")]
    pub fn item(&self, metric: Metric) -> Option<&MenuBarItem> {
        self.items.iter().find(|item| item.metric == metric)
    }

    /// 某个指标当前的版式和展示内容。
    #[cfg(target_os = "macos")]
    pub fn look(&self, metric: Metric) -> (ItemStyle, Show) {
        self.item(metric)
            .map_or_else(Default::default, |i| (i.style, i.show))
    }

    pub fn move_item(&mut self, metric: Metric, up: bool) {
        let Some(index) = self.items.iter().position(|i| i.metric == metric) else {
            return;
        };
        let target = if up {
            index.checked_sub(1)
        } else {
            index.checked_add(1).filter(|&i| i < self.items.len())
        };
        if let Some(target) = target {
            self.items.swap(index, target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_preserves_order_and_restores_new_metrics() {
        let mut config: MenuBarSettings = serde_json::from_str(r#"{"items":[{"metric":"memory","enabled":false},{"metric":"cpu"},{"metric":"memory"}],"refresh_ms":0,"process_rows":999}"#).unwrap();
        config.normalize();
        assert_eq!(config.items.len(), 7);
        assert_eq!(config.items[0].metric, Metric::Memory);
        assert!(!config.items[0].enabled);
        assert_eq!(config.items[1].metric, Metric::Cpu);
        assert_eq!(config.refresh_ms, 1000);
        assert_eq!(config.process_rows, 8);
        assert!(config.show_units);
        // 旧版本的设置文件里没有这两项，读进来要是默认值，而不是 0 / false。
        assert_eq!(config.spacing, DEFAULT_SPACING);
        assert!(config.show_icon);
    }

    #[test]
    fn spacing_is_clamped_and_the_icon_never_disappears_with_nothing_else() {
        let mut config: MenuBarSettings =
            serde_json::from_str(r#"{"spacing":999,"show_icon":false}"#).unwrap();
        config.normalize();
        assert_eq!(config.spacing, *SPACING_RANGE.end());
        assert_eq!(config.padding(), 12.);
        // 有监控项在，图标可以藏起来。
        assert!(!config.icon_visible());
        // 全部监控项都关了，或者干脆不在菜单栏上显示监控，图标必须回来。
        for item in &mut config.items {
            item.enabled = false;
        }
        assert!(config.icon_visible());
        config.items[0].enabled = true;
        config.enabled = false;
        assert!(config.icon_visible());
    }

    #[test]
    fn reorder_and_roundtrip_preserve_per_item_choices() {
        let mut config = MenuBarSettings::default();
        config.items[1].style = ItemStyle::Graph;
        config.items[1].show = Show::Free;
        config.move_item(Metric::Memory, true);
        config.move_item(Metric::Memory, true);
        assert_eq!(config.items[0].metric, Metric::Memory);
        let restored: MenuBarSettings =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored, config);
    }

    #[test]
    fn old_settings_keep_working() {
        // 旧版本写下的 `compact` 样式和已经删除的 `in_app` 都不能让设置读不出来。
        let mut config: MenuBarSettings = serde_json::from_str(
            r#"{"in_app":true,"items":[{"metric":"cpu","style":"compact","show":"free"}]}"#,
        )
        .unwrap();
        assert_eq!(config.items[0].style, ItemStyle::Inline);
        // CPU 没有「剩余」这种读数，回到默认。
        config.normalize();
        assert_eq!(config.items[0].show, Show::Auto);
    }
}
