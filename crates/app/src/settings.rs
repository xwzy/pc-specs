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
    pub alerts: bool,
    pub server_enabled: bool,
    pub server_lan: bool,
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
            server_enabled: true,
            // 与旧版保持一致：默认监听所有网卡，便于其他机器拉取指标。
            server_lan: true,
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

    /// 把手工改坏的值收敛回合法范围。
    fn sanitized(mut self) -> Self {
        self.menu_bar.normalize();
        self.bigscreen.normalize();
        self.interval_ms = self.interval_ms.clamp(250, 10_000);
        if !matches!(self.history_minutes, 1 | 5 | 10) {
            self.history_minutes = 5;
        }
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
}
