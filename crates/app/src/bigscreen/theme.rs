//! 监控大屏的主题。
//!
//! gpui-component 的主题是全局的（所有窗口共用一份），而大屏要有自己的一套视觉，
//! 所以这里单独定义调色板，大屏里的颜色全部从 `Palette` 取，不碰全局主题。

use super::config::ChartPref;
use crate::health::{self, Level};
use gpui_kit::{Background, Hsla, linear_color_stop, linear_gradient, rgb};
use rust_i18n::t;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeId {
    #[default]
    Midnight,
    Aurora,
    Cyberpunk,
    Matrix,
    Amber,
    Graphite,
    Oled,
    Nord,
    Dracula,
    Sunset,
    Ocean,
    Blueprint,
    Paper,
    Mint,
    Sakura,
}

/// 趋势图的画法。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartStyle {
    Area,
    Line,
    Bars,
}

impl ChartPref {
    pub fn resolve(self, theme: ChartStyle) -> ChartStyle {
        match self {
            Self::Auto => theme,
            Self::Area => ChartStyle::Area,
            Self::Line => ChartStyle::Line,
            Self::Bars => ChartStyle::Bars,
        }
    }
}

/// 背景上的装饰网格，步长是设计尺寸（1280×720 的屏幕上）里的像素。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grid {
    None,
    Lines(f32),
    Dots(f32),
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg: [Hsla; 2],
    /// 背景渐变方向（度），0 是从下往上，180 是从上往下。
    pub bg_angle: f32,
    pub tile: Hsla,
    pub tile_edge: Hsla,
    /// 磁贴边框宽度（px）。
    pub edge_width: f32,
    pub text: Hsla,
    pub muted: Hsla,
    /// 轨道、网格线、分隔线这类极淡的线条。
    pub faint: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub danger: Hsla,
    pub cpu: Hsla,
    pub mem: Hsla,
    pub gpu: Hsla,
    pub down: Hsla,
    pub up: Hsla,
    pub disk: Hsla,
    pub temp: Hsla,
    pub battery: Hsla,
    pub grid: Grid,
    pub grid_color: Hsla,
    /// 磁贴圆角（设计像素）。
    pub radius: f32,
    /// 霓虹发光强度，0 表示不发光。
    pub glow: f32,
    /// 全部文字用等宽字体。
    pub mono: bool,
    pub chart: ChartStyle,
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hexa(value: u32, alpha: f32) -> Hsla {
    hex(value).opacity(alpha)
}

/// 深色主题的共同底色；各主题只改自己不同的地方。
fn dark() -> Palette {
    Palette {
        dark: true,
        bg: [hex(0x0b1020); 2],
        bg_angle: 160.,
        tile: hexa(0xffffff, 0.05),
        tile_edge: hexa(0xffffff, 0.10),
        edge_width: 1.,
        text: hex(0xf2f5ff),
        muted: hexa(0xffffff, 0.55),
        faint: hexa(0xffffff, 0.08),
        ok: hex(0x34d399),
        warn: hex(0xfbbf24),
        danger: hex(0xf87171),
        cpu: hex(0x60a5fa),
        mem: hex(0x34d399),
        gpu: hex(0xc084fc),
        down: hex(0x22d3ee),
        up: hex(0xf472b6),
        disk: hex(0xfbbf24),
        temp: hex(0xfb923c),
        battery: hex(0x4ade80),
        grid: Grid::None,
        grid_color: hexa(0xffffff, 0.05),
        radius: 14.,
        glow: 0.,
        mono: false,
        chart: ChartStyle::Area,
    }
}

fn light() -> Palette {
    Palette {
        dark: false,
        bg: [hex(0xf5f2ea); 2],
        bg_angle: 160.,
        tile: hex(0xffffff),
        tile_edge: hexa(0x000000, 0.08),
        edge_width: 1.,
        text: hex(0x1f2430),
        muted: hex(0x7a7f8c),
        faint: hexa(0x000000, 0.07),
        ok: hex(0x16a34a),
        warn: hex(0xd97706),
        danger: hex(0xdc2626),
        cpu: hex(0x2563eb),
        mem: hex(0x16a34a),
        gpu: hex(0x9333ea),
        down: hex(0x0891b2),
        up: hex(0xdb2777),
        disk: hex(0xd97706),
        temp: hex(0xea580c),
        battery: hex(0x16a34a),
        grid: Grid::None,
        grid_color: hexa(0x000000, 0.04),
        radius: 16.,
        glow: 0.,
        mono: false,
        chart: ChartStyle::Area,
    }
}

impl ThemeId {
    pub const ALL: [Self; 15] = [
        Self::Midnight,
        Self::Aurora,
        Self::Cyberpunk,
        Self::Matrix,
        Self::Amber,
        Self::Graphite,
        Self::Oled,
        Self::Nord,
        Self::Dracula,
        Self::Sunset,
        Self::Ocean,
        Self::Blueprint,
        Self::Paper,
        Self::Mint,
        Self::Sakura,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Midnight => "midnight",
            Self::Aurora => "aurora",
            Self::Cyberpunk => "cyberpunk",
            Self::Matrix => "matrix",
            Self::Amber => "amber",
            Self::Graphite => "graphite",
            Self::Oled => "oled",
            Self::Nord => "nord",
            Self::Dracula => "dracula",
            Self::Sunset => "sunset",
            Self::Ocean => "ocean",
            Self::Blueprint => "blueprint",
            Self::Paper => "paper",
            Self::Mint => "mint",
            Self::Sakura => "sakura",
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Midnight => t!("bs_theme_midnight"),
            Self::Aurora => t!("bs_theme_aurora"),
            Self::Cyberpunk => t!("bs_theme_cyberpunk"),
            Self::Matrix => t!("bs_theme_matrix"),
            Self::Amber => t!("bs_theme_amber"),
            Self::Graphite => t!("bs_theme_graphite"),
            Self::Oled => t!("bs_theme_oled"),
            Self::Nord => t!("bs_theme_nord"),
            Self::Dracula => t!("bs_theme_dracula"),
            Self::Sunset => t!("bs_theme_sunset"),
            Self::Ocean => t!("bs_theme_ocean"),
            Self::Blueprint => t!("bs_theme_blueprint"),
            Self::Paper => t!("bs_theme_paper"),
            Self::Mint => t!("bs_theme_mint"),
            Self::Sakura => t!("bs_theme_sakura"),
        }
        .into_owned()
    }

    /// 按 `ALL` 的顺序循环切换。
    pub fn step(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|id| *id == self).unwrap_or(0);
        let count = Self::ALL.len();
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        Self::ALL[next]
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Midnight => Palette {
                bg: [hex(0x0a1020), hex(0x0f1c3f)],
                tile: hexa(0x1b2a5c, 0.42),
                tile_edge: hexa(0x7f9bff, 0.20),
                text: hex(0xe8eefc),
                muted: hex(0x8d9cc6),
                faint: hexa(0x8fa6ff, 0.11),
                glow: 0.3,
                radius: 16.,
                ..dark()
            },
            Self::Aurora => Palette {
                bg: [hex(0x04131a), hex(0x0b2f38)],
                bg_angle: 135.,
                tile: hexa(0xffffff, 0.06),
                tile_edge: hexa(0xb6fff0, 0.18),
                text: hex(0xeafff9),
                muted: hex(0x8fbdb4),
                faint: hexa(0xb6fff0, 0.11),
                cpu: hex(0x5eead4),
                mem: hex(0xa78bfa),
                gpu: hex(0xf0abfc),
                down: hex(0x67e8f9),
                up: hex(0xfda4af),
                disk: hex(0x86efac),
                temp: hex(0xfdba74),
                battery: hex(0x86efac),
                radius: 22.,
                glow: 0.5,
                ..dark()
            },
            Self::Cyberpunk => Palette {
                bg: [hex(0x07040f), hex(0x1d0b36)],
                bg_angle: 150.,
                tile: hexa(0x1a0b2e, 0.78),
                tile_edge: hexa(0xff2bd6, 0.50),
                edge_width: 1.5,
                text: hex(0xf7ecff),
                muted: hex(0xa98fd1),
                faint: hexa(0xff2bd6, 0.14),
                ok: hex(0x7cff6b),
                warn: hex(0xfcee09),
                danger: hex(0xff3860),
                cpu: hex(0x00f0ff),
                mem: hex(0xff2bd6),
                gpu: hex(0xfcee09),
                down: hex(0x00f0ff),
                up: hex(0xff2bd6),
                disk: hex(0x7cff6b),
                temp: hex(0xff6a3d),
                battery: hex(0x7cff6b),
                grid: Grid::Lines(48.),
                grid_color: hexa(0xff2bd6, 0.07),
                radius: 6.,
                glow: 1.,
                ..dark()
            },
            Self::Matrix => Palette {
                bg: [hex(0x000000), hex(0x00140a)],
                tile: hexa(0x00ff41, 0.04),
                tile_edge: hexa(0x00ff41, 0.38),
                text: hex(0x8cff9f),
                muted: hex(0x2f9e55),
                faint: hexa(0x00ff41, 0.11),
                ok: hex(0x00ff41),
                warn: hex(0xe6ff00),
                danger: hex(0xff3b3b),
                cpu: hex(0x00ff41),
                mem: hex(0x7dff9b),
                gpu: hex(0x39ff14),
                down: hex(0x00ff41),
                up: hex(0xb6ffc4),
                disk: hex(0x00cc33),
                temp: hex(0xd7ff3a),
                battery: hex(0x00ff41),
                grid: Grid::Dots(22.),
                grid_color: hexa(0x00ff41, 0.11),
                radius: 2.,
                glow: 0.8,
                mono: true,
                chart: ChartStyle::Bars,
                ..dark()
            },
            Self::Amber => Palette {
                bg: [hex(0x080500), hex(0x150c00)],
                tile: hexa(0xffb000, 0.05),
                tile_edge: hexa(0xffb000, 0.38),
                text: hex(0xffc04d),
                muted: hex(0xb07a14),
                faint: hexa(0xffb000, 0.11),
                ok: hex(0xffd166),
                warn: hex(0xff9f1a),
                danger: hex(0xff4d2e),
                cpu: hex(0xffb000),
                mem: hex(0xffd166),
                gpu: hex(0xff8c00),
                down: hex(0xffcc33),
                up: hex(0xff9f1a),
                disk: hex(0xffe08a),
                temp: hex(0xff6a00),
                battery: hex(0xffd166),
                grid: Grid::Lines(32.),
                grid_color: hexa(0xffb000, 0.05),
                radius: 2.,
                glow: 0.7,
                mono: true,
                chart: ChartStyle::Bars,
                ..dark()
            },
            Self::Graphite => Palette {
                bg: [hex(0x111214), hex(0x1a1c20)],
                tile: hex(0x1e2025),
                tile_edge: hexa(0xffffff, 0.07),
                text: hex(0xf1f2f4),
                muted: hex(0x8c9099),
                faint: hexa(0xffffff, 0.07),
                cpu: hex(0x7aa2f7),
                mem: hex(0x9ece6a),
                gpu: hex(0xbb9af7),
                down: hex(0x7dcfff),
                up: hex(0xf7768e),
                disk: hex(0xe0af68),
                temp: hex(0xff9e64),
                radius: 12.,
                ..dark()
            },
            Self::Oled => Palette {
                bg: [hex(0x000000); 2],
                tile: hexa(0xffffff, 0.035),
                tile_edge: hexa(0xffffff, 0.10),
                text: hex(0xffffff),
                muted: hex(0x80808a),
                faint: hexa(0xffffff, 0.08),
                cpu: hex(0x4da3ff),
                mem: hex(0x3ddc84),
                gpu: hex(0xb388ff),
                down: hex(0x4dd0e1),
                up: hex(0xff80ab),
                disk: hex(0xffd54f),
                temp: hex(0xff8a65),
                radius: 18.,
                glow: 0.15,
                ..dark()
            },
            Self::Nord => Palette {
                bg: [hex(0x2b303b), hex(0x2e3440)],
                tile: hex(0x3b4252),
                tile_edge: hex(0x4c566a),
                text: hex(0xeceff4),
                muted: hex(0xa3adc2),
                faint: hexa(0xd8dee9, 0.10),
                ok: hex(0xa3be8c),
                warn: hex(0xebcb8b),
                danger: hex(0xbf616a),
                cpu: hex(0x88c0d0),
                mem: hex(0xa3be8c),
                gpu: hex(0xb48ead),
                down: hex(0x8fbcbb),
                up: hex(0xd08770),
                disk: hex(0xebcb8b),
                temp: hex(0xd08770),
                battery: hex(0xa3be8c),
                radius: 10.,
                ..dark()
            },
            Self::Dracula => Palette {
                bg: [hex(0x1b1c26), hex(0x282a36)],
                tile: hex(0x2f3242),
                tile_edge: hexa(0xbd93f9, 0.28),
                text: hex(0xf8f8f2),
                muted: hex(0x9aa0c8),
                faint: hexa(0xbd93f9, 0.12),
                ok: hex(0x50fa7b),
                warn: hex(0xf1fa8c),
                danger: hex(0xff5555),
                cpu: hex(0xbd93f9),
                mem: hex(0x50fa7b),
                gpu: hex(0xff79c6),
                down: hex(0x8be9fd),
                up: hex(0xf1fa8c),
                disk: hex(0xffb86c),
                temp: hex(0xff5555),
                battery: hex(0x50fa7b),
                radius: 12.,
                glow: 0.2,
                ..dark()
            },
            Self::Sunset => Palette {
                bg: [hex(0x1a0b2e), hex(0x5b1f3a)],
                bg_angle: 170.,
                tile: hexa(0xffffff, 0.07),
                tile_edge: hexa(0xffd0b0, 0.22),
                text: hex(0xfff3ea),
                muted: hex(0xe0b4aa),
                faint: hexa(0xffffff, 0.11),
                ok: hex(0x7ee0a6),
                warn: hex(0xff9f43),
                danger: hex(0xff4d6d),
                cpu: hex(0xffb86b),
                mem: hex(0xff7a8a),
                gpu: hex(0xf9a8d4),
                down: hex(0xffd166),
                up: hex(0xff8fab),
                disk: hex(0xfcd34d),
                temp: hex(0xff6b57),
                battery: hex(0xffd166),
                radius: 20.,
                glow: 0.4,
                ..dark()
            },
            Self::Ocean => Palette {
                bg: [hex(0x021526), hex(0x07507a)],
                tile: hexa(0xffffff, 0.07),
                tile_edge: hexa(0x9ee7ff, 0.24),
                text: hex(0xe9f7ff),
                muted: hex(0x8fc3dc),
                faint: hexa(0x9ee7ff, 0.12),
                cpu: hex(0x38bdf8),
                mem: hex(0x2dd4bf),
                gpu: hex(0xa5b4fc),
                down: hex(0x22d3ee),
                up: hex(0xfb7185),
                disk: hex(0xfde68a),
                temp: hex(0xfb923c),
                battery: hex(0x4ade80),
                radius: 18.,
                glow: 0.35,
                ..dark()
            },
            Self::Blueprint => Palette {
                bg: [hex(0x0a3d91), hex(0x0b2f75)],
                tile: hexa(0xffffff, 0.0),
                tile_edge: hexa(0xffffff, 0.55),
                edge_width: 1.5,
                text: hex(0xffffff),
                muted: hex(0xb7ceff),
                faint: hexa(0xffffff, 0.18),
                ok: hex(0xd7f5c4),
                warn: hex(0xffe08a),
                danger: hex(0xff9aa2),
                cpu: hex(0xffffff),
                mem: hex(0xbfe0ff),
                gpu: hex(0xffe08a),
                down: hex(0x9ee7ff),
                up: hex(0xffc2d9),
                disk: hex(0xd7f5c4),
                temp: hex(0xffb38a),
                battery: hex(0xd7f5c4),
                grid: Grid::Lines(36.),
                grid_color: hexa(0xffffff, 0.09),
                radius: 0.,
                mono: true,
                chart: ChartStyle::Line,
                ..dark()
            },
            Self::Paper => Palette {
                bg: [hex(0xf5f2ea), hex(0xece7da)],
                tile: hex(0xfffdf8),
                tile_edge: hex(0xe0d9c7),
                text: hex(0x1f2430),
                muted: hex(0x7a7f8c),
                ..light()
            },
            Self::Mint => Palette {
                bg: [hex(0xeafaf3), hex(0xd6f1e6)],
                tile: hex(0xffffff),
                tile_edge: hex(0xc6e9da),
                text: hex(0x0f3a2f),
                muted: hex(0x5b8a7b),
                faint: hexa(0x0f3a2f, 0.08),
                cpu: hex(0x0d9488),
                down: hex(0x0284c7),
                up: hex(0xe11d48),
                disk: hex(0xca8a04),
                gpu: hex(0x7c3aed),
                radius: 20.,
                ..light()
            },
            Self::Sakura => Palette {
                bg: [hex(0xfff3f7), hex(0xfde0eb)],
                tile: hex(0xffffff),
                tile_edge: hex(0xf6cadb),
                text: hex(0x4a1d2e),
                muted: hex(0xa0657f),
                faint: hexa(0x4a1d2e, 0.08),
                cpu: hex(0xe11d73),
                gpu: hex(0x8b5cf6),
                down: hex(0x0ea5e9),
                up: hex(0xf43f5e),
                temp: hex(0xf97316),
                radius: 22.,
                ..light()
            },
        }
    }
}

impl Palette {
    pub fn background(&self) -> Background {
        linear_gradient(
            self.bg_angle,
            linear_color_stop(self.bg[0], 0.),
            linear_color_stop(self.bg[1], 1.),
        )
    }

    pub fn level(&self, level: Level) -> Hsla {
        match level {
            Level::Healthy => self.ok,
            Level::Warning => self.warn,
            Level::Critical => self.danger,
        }
    }

    /// 平时用组件自己的主色，到了警戒线换成警告 / 危险色。
    pub fn graded(&self, level: Level, base: Hsla) -> Hsla {
        match level {
            Level::Healthy => base,
            Level::Warning => self.warn,
            Level::Critical => self.danger,
        }
    }

    /// 占用率的颜色。
    pub fn usage(&self, percent: f32, base: Hsla) -> Hsla {
        self.graded(health::usage_level(percent), base)
    }
}
