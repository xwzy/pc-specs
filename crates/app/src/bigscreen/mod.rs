//! 监控大屏：一块可以放到另一台显示器上的实时看板。
//!
//! - `config`：设置模型（放在哪块屏、哪些组件、什么主题……）。
//! - `theme`：预置主题的调色板。
//! - `layout`：自适应排版，横屏、竖屏、超宽屏都把组件排满整个屏幕。
//! - `data`：大屏自己的走势历史，以及「这台机器上有没有这个硬件」的判断。
//! - `paint` / `widgets` / `tiles`：绘图基元、磁贴外壳、各组件的内容。
//! - `view`：窗口里的画面；`preferences`：设置页（带实时预览）；`awake`：大屏开着时不让屏幕熄灭。
//!
//! 本文件负责窗口本身：选屏、打开 / 关闭、全屏、快捷键。

mod awake;
pub mod config;
pub mod data;
mod layout;
mod paint;
pub mod preferences;
mod theme;
mod tiles;
mod view;
mod widgets;

pub use view::remember;

use crate::state::AppState;
use config::BigScreenSettings;
use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext as _, Bounds, DisplayId, KeyBinding, Pixels, WindowBounds, WindowOptions,
    actions, point, px, size,
};
use pc_specs_core::util::guarded;
use rust_i18n::t;

actions!(
    bigscreen,
    [
        OpenBigScreen,
        ToggleFullscreen,
        NextTheme,
        PreviousTheme,
        ShowSettings,
        ZoomIn,
        ZoomOut,
        ToggleHeader,
        Leave
    ]
);

/// 大屏窗口里的快捷键只在它自己获得焦点时生效，免得抢了别的窗口里的按键。
const KEY_CONTEXT: &str = "BigScreen";
const APP_ID: &str = "com.xwzy.pc-specs";
/// 窗口模式下第一次打开的大小。
const DEFAULT_SIZE: (f32, f32) = (1280., 720.);
const MIN_SIZE: (f32, f32) = (480., 320.);

pub fn init(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-shift-b", OpenBigScreen, None)]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([KeyBinding::new("ctrl-shift-b", OpenBigScreen, None)]);
    let wall = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("f", ToggleFullscreen, wall),
        KeyBinding::new("f11", ToggleFullscreen, wall),
        KeyBinding::new("escape", Leave, wall),
        KeyBinding::new("right", NextTheme, wall),
        KeyBinding::new("t", NextTheme, wall),
        KeyBinding::new("left", PreviousTheme, wall),
        KeyBinding::new("=", ZoomIn, wall),
        KeyBinding::new("+", ZoomIn, wall),
        KeyBinding::new("-", ZoomOut, wall),
        KeyBinding::new("h", ToggleHeader, wall),
        KeyBinding::new("s", ShowSettings, wall),
    ]);
    crate::workspace::on_action(cx, |_: &OpenBigScreen, cx| open(cx));
}

/// 一块显示器。
pub struct Screen {
    pub id: DisplayId,
    /// 重启和插拔之后还认得出来的标识，存在设置里。
    pub key: String,
    pub width: f32,
    pub height: f32,
    pub primary: bool,
    bounds: Bounds<Pixels>,
}

/// 现在接着的所有显示器。休眠或者正在插拔时可能一块都没有。
pub fn screens(cx: &App) -> Vec<Screen> {
    if !crate::workspace::screens_available(cx) {
        return Vec::new();
    }
    let primary = cx.primary_display().map(|display| display.id());
    cx.displays()
        .iter()
        .enumerate()
        .map(|(index, display)| {
            let bounds = display.bounds();
            Screen {
                id: display.id(),
                key: display
                    .uuid()
                    .map(|uuid| uuid.to_string())
                    .unwrap_or_else(|_| format!("display-{}", index + 1)),
                width: bounds.size.width.as_f32(),
                height: bounds.size.height.as_f32(),
                primary: Some(display.id()) == primary,
                bounds,
            }
        })
        .collect()
}

/// 大屏该上哪块屏：设置里指定的优先（拔掉了就回落），其次是不是主屏的那块，最后是主屏。
pub fn pick<'a>(cfg: &BigScreenSettings, screens: &'a [Screen]) -> Option<&'a Screen> {
    cfg.display
        .as_ref()
        .and_then(|key| screens.iter().find(|screen| &screen.key == key))
        .or_else(|| screens.iter().find(|screen| !screen.primary))
        .or_else(|| screens.iter().find(|screen| screen.primary))
        .or_else(|| screens.first())
}

/// 大屏窗口现在开着没有。
pub fn is_open(cx: &App) -> bool {
    AppState::global(cx)
        .read(cx)
        .wall_window
        .is_some_and(|handle| cx.windows().contains(&handle))
}

/// 打开大屏；已经开着就把它带到最前面。
pub fn open(cx: &mut App) {
    // 菜单、托盘和快捷键都由系统回调直接驱动，里面 panic 会让整个进程 abort。
    guarded("open big screen", || show(cx));
}

fn show(cx: &mut App) {
    let state = AppState::global(cx);
    if let Some(handle) = state.read(cx).wall_window
        && cx.windows().contains(&handle)
    {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
        return;
    }
    let screens = screens(cx);
    if screens.is_empty() {
        tracing::warn!("no display available, not opening the big screen");
        return;
    }
    let cfg = state.read(cx).settings.bigscreen.clone();
    let options = window_options(&cfg, pick(&cfg, &screens), cx);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(&format!("{} · {}", t!("app_name"), t!("bs_title")));
        let state = state.clone();
        cx.new(|cx| view::BigScreen::new(state, window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            state.update(cx, |s, cx| s.wall_opened(handle, cx));
            cx.activate(true);
        }
        Err(e) => tracing::error!("failed to open the big screen: {e}"),
    }
}

/// 关闭大屏，位置和大小照常记下来。
pub fn close(cx: &mut App) {
    guarded("close big screen", || {
        let state = AppState::global(cx);
        let Some(handle) = state
            .read(cx)
            .wall_window
            .filter(|handle| cx.windows().contains(handle))
        else {
            return;
        };
        let _ = handle.update(cx, |_, window, cx| {
            remember(&state, window, cx);
            window.remove_window();
        });
        state.read(cx).settings.save();
    });
}

/// 换了屏幕或者全屏设置之后，让开着的大屏按新设置重新开一遍；没开着就什么也不做。
pub fn reopen(cx: &mut App) {
    if is_open(cx) {
        close(cx);
        open(cx);
    }
}

/// 设置了「启动时打开」就打开。
pub fn open_on_launch(cx: &mut App) {
    if AppState::global(cx)
        .read(cx)
        .settings
        .bigscreen
        .open_on_launch
    {
        open(cx);
    }
}

fn window_options(cfg: &BigScreenSettings, screen: Option<&Screen>, cx: &App) -> WindowOptions {
    let restore = restore_bounds(cfg, screen, cx);
    let window_bounds = if cfg.fullscreen {
        WindowBounds::Fullscreen(restore)
    } else {
        WindowBounds::Windowed(restore)
    };
    // macOS 上标题栏透明、红绿灯压在顶栏上；其它平台用系统自己的标题栏。
    let base = if cfg!(target_os = "macos") {
        TitleBar::window_options()
    } else {
        WindowOptions::default()
    };
    WindowOptions {
        window_bounds: Some(window_bounds),
        display_id: screen.map(|screen| screen.id),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        app_id: Some(APP_ID.into()),
        ..base
    }
}

/// 窗口模式下的位置：上次的位置还完整地落在目标屏幕上就沿用，否则在那块屏上居中。
/// 全屏时它是退出全屏之后窗口回到的位置。
fn restore_bounds(cfg: &BigScreenSettings, screen: Option<&Screen>, cx: &App) -> Bounds<Pixels> {
    if let (Some(saved), Some(screen)) = (cfg.window, screen) {
        let bounds = Bounds::new(
            point(px(saved.x), px(saved.y)),
            size(px(saved.width), px(saved.height)),
        );
        // `Bounds::contains` 不含右、下边缘，铺满屏幕的窗口会被误判成「不在屏幕上」，所以自己比。
        let (left, top) = (screen.bounds.origin.x, screen.bounds.origin.y);
        let corner = bounds.bottom_right();
        let inside = bounds.origin.x >= left
            && bounds.origin.y >= top
            && corner.x <= left + screen.bounds.size.width
            && corner.y <= top + screen.bounds.size.height;
        if inside && saved.width >= MIN_SIZE.0 && saved.height >= MIN_SIZE.1 {
            return bounds;
        }
    }
    // 默认尺寸不能超过屏幕。
    let (width, height) = match screen {
        Some(screen) => (
            DEFAULT_SIZE.0.min(screen.width - 80.),
            DEFAULT_SIZE.1.min(screen.height - 120.),
        ),
        None => DEFAULT_SIZE,
    };
    Bounds::centered(
        screen.map(|screen| screen.id),
        size(px(width.max(MIN_SIZE.0)), px(height.max(MIN_SIZE.1))),
        cx,
    )
}
