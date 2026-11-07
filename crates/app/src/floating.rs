//! 悬浮网速小窗：始终置顶的小条，显示实时下载 / 上传速度。
//!
//! 左键按住拖动，双击或右键关闭。关闭后设置里的开关由 `workspace::on_window_closed` 同步回「关」。
//! 位置会记下来，下次打开还在原处。

use crate::fmt;
use crate::state::AppState;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, Styled as _, Subscription, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div, point, px, size,
};

const WIDTH: f32 = 168.0;
const HEIGHT: f32 = 38.0;
/// 离屏幕右 / 下边缘的距离；下边留得多一点，避开任务栏 / Dock。
const MARGIN: f32 = 24.0;
const MARGIN_BOTTOM: f32 = 72.0;

/// 记下悬浮窗现在的位置。
pub fn remember(window: &Window, cx: &mut App) {
    let origin = window.bounds().origin;
    AppState::global(cx).update(cx, |s, _| {
        s.settings.floating_pos = Some((origin.x.as_f32(), origin.y.as_f32()));
    });
}

/// 关闭悬浮窗。
pub fn close(handle: AnyWindowHandle, cx: &mut App) {
    let _ = handle.update(cx, |_, window, cx| {
        remember(window, cx);
        window.remove_window();
    });
}

/// 打开悬浮窗并返回句柄：优先用上次的位置，那个位置已经不在任何显示器上就放回右下角。
///
/// 调用方通常正处在 `AppState` 自己的 update 里，所以这里只拿实体句柄，绝不读 / 写它。
pub fn open(
    state: Entity<AppState>,
    saved: Option<(f32, f32)>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    if !crate::workspace::screens_available(cx) {
        tracing::warn!("no display available, not opening the floating widget");
        return None;
    }
    let area = cx
        .primary_display()
        .map(|display| display.bounds())
        .unwrap_or_else(|| Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(800.))));
    let corner = point(
        area.right() - px(WIDTH + MARGIN),
        area.bottom() - px(HEIGHT + MARGIN_BOTTOM),
    );
    let origin = saved
        .map(|(x, y)| point(px(x), px(y)))
        .filter(|origin| {
            let far = point(origin.x + px(WIDTH), origin.y + px(HEIGHT));
            cx.displays()
                .iter()
                .any(|d| d.bounds().contains(origin) && d.bounds().contains(&far))
        })
        .unwrap_or(corner);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(WIDTH), px(HEIGHT)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |_, cx| {
        cx.new(|cx| FloatingWidget::new(state, cx))
    }) {
        Ok((handle, _)) => Some(handle),
        Err(e) => {
            tracing::error!("failed to open floating widget: {e}");
            None
        }
    }
}

struct FloatingWidget {
    state: Entity<AppState>,
    _subscription: Subscription,
}

impl FloatingWidget {
    fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}

impl Render for FloatingWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rx, tx, unit) = {
            let s = self.state.read(cx);
            let (rx, tx) = s
                .latest
                .as_ref()
                .map(|t| (t.net_rx_bps, t.net_tx_bps))
                .unwrap_or_default();
            (rx, tx, s.settings.net_unit)
        };
        h_flex()
            .id("floating-net")
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_sm()
            .font_semibold()
            .child(
                div()
                    .text_color(cx.theme().cyan)
                    .child(format!("↓ {}", fmt::rate_compact(rx, unit))),
            )
            .child(
                div()
                    .text_color(cx.theme().magenta)
                    .child(format!("↑ {}", fmt::rate_compact(tx, unit))),
            )
            .on_mouse_down(MouseButton::Left, |event, window, cx| {
                if event.click_count >= 2 {
                    remember(window, cx);
                    window.remove_window();
                } else {
                    window.start_window_move();
                }
            })
            .on_mouse_down(MouseButton::Right, |_, window, cx| {
                remember(window, cx);
                window.remove_window();
            })
    }
}
