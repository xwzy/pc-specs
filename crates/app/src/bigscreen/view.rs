//! 大屏窗口里的画面：背景、顶栏、磁贴，以及鼠标一动就浮出来的工具条。
//!
//! 画面本身（`wall`）和窗口分开：设置页的预览调用的是同一个函数，只是画布更小，所以所见即所得。

use super::awake::Awake;
use super::config::{BigScreenSettings, SCALE_RANGE, SCALE_STEP, WidgetKind};
use super::layout::{self, Cell, Rect};
use super::theme::Palette;
use super::widgets::Ctx;
use super::{
    KEY_CONTEXT, Leave, NextTheme, PreviousTheme, ShowSettings, ToggleFullscreen, ToggleHeader,
    ZoomIn, ZoomOut, data, paint, tiles,
};
use crate::health::{self, Level};
use crate::settings::WindowState;
use crate::state::{AppState, Page};
use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, BoxShadow, ClickEvent, Context, Div, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent, ParentElement as _, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    WindowBounds, div, hsla, prelude::FluentBuilder as _, px,
};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::time::{Duration, Instant};

/// 顶栏的设计高度（1280×720 的屏幕上）。
const HEADER: f32 = 46.;
/// 磁贴里的字缩到这个比例以下就看不清了，屏幕再小就从末尾开始舍弃组件。
const MIN_SCALE: f32 = 0.62;
/// macOS 窗口模式下，红绿灯占掉顶栏左边的宽度。
const TRAFFIC_LIGHTS: f32 = 76.;
/// macOS 窗口模式下，标题栏（红绿灯所在的那一条）的高度。
const TITLEBAR: f32 = 28.;
/// 鼠标最后一次动过之后，工具条再留多久。
const TOOLBAR_FOR: Duration = Duration::from_secs(3);

/// 画布：按 `width × height`（逻辑像素）排版，再整体乘上 `factor` 画出来。
/// 真实的大屏窗口 `factor` 是 1；设置页的预览把一整块屏幕缩进一个小框，
/// 所以排法、字号比例和真实大屏完全一致。
#[derive(Clone, Copy)]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
    pub factor: f32,
}

/// 与磁贴无关的外围尺寸（排版用的逻辑像素）：顶栏、边距、磁贴区。
struct Frame {
    /// 顶栏等装饰元素的缩放：以 1280×720 为 1。
    chrome: f32,
    gap: f32,
    header: f32,
    area: Rect,
}

impl Frame {
    fn new(cfg: &BigScreenSettings, width: f32, height: f32) -> Self {
        let chrome = (width / 1280.).min(height / 720.).clamp(0.6, 2.6);
        let gap = cfg.density.gap() * chrome;
        let header = if cfg.show_header { HEADER * chrome } else { 0. };
        let top = if cfg.show_header {
            gap + header + gap
        } else {
            gap
        };
        Self {
            chrome,
            gap,
            header,
            area: Rect {
                x: gap,
                y: top,
                w: (width - 2. * gap).max(1.),
                h: (height - top - gap).max(1.),
            },
        }
    }
}

/// 整张大屏。`inset` 是顶栏左边要让出来的宽度（屏幕像素），`mono` 非空时全部文字用这个字体。
pub fn wall(
    app: &AppState,
    cfg: &BigScreenSettings,
    canvas: Canvas,
    inset: f32,
    mono: Option<SharedString>,
) -> Div {
    let Canvas {
        width,
        height,
        factor,
    } = canvas;
    let pal = cfg.theme.palette();
    let frame = Frame::new(cfg, width, height);

    let wanted: Vec<(WidgetKind, Cell)> = cfg
        .widgets
        .iter()
        .filter(|widget| widget.enabled && (!cfg.hide_missing || data::available(widget.kind, app)))
        .map(|widget| {
            (
                widget.kind,
                Cell {
                    w: widget.w,
                    h: widget.h,
                },
            )
        })
        .collect();
    let cells: Vec<Cell> = wanted.iter().map(|(_, cell)| *cell).collect();
    let arrangement = layout::fit(
        &cells,
        frame.area.w,
        frame.area.h,
        frame.gap,
        cfg.scale as f32 / 100.,
        MIN_SCALE,
    );
    let hidden = wanted.len() - arrangement.rects.len();

    let ctx = Ctx {
        app,
        cfg,
        pal,
        style: cfg.chart.resolve(pal.chart),
        k: frame.chrome * factor,
        now: chrono::Local::now(),
    };

    let mut root = div()
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(pal.background())
        .text_color(pal.text)
        .when_some(mono, |root, family| root.font_family(family))
        .child(paint::backdrop(pal.grid, pal.grid_color, ctx.k));

    if cfg.show_header {
        let rect = Rect {
            x: frame.gap * factor,
            y: frame.gap * factor,
            w: (width - 2. * frame.gap) * factor,
            h: frame.header * factor,
        };
        if let Some(bar) = guarded("render header", || top_bar(&ctx, rect, inset, hidden)) {
            root = root.child(bar);
        }
    }

    if wanted.is_empty() {
        return root.child(
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(ctx.text(t!("bs_empty").into_owned(), 16., pal.muted)),
        );
    }

    for ((kind, _), rect) in wanted.iter().zip(&arrangement.rects) {
        let scaled = Rect {
            x: (frame.area.x + rect.x) * factor,
            y: (frame.area.y + rect.y) * factor,
            w: rect.w * factor,
            h: rect.h * factor,
        };
        let tile_ctx = ctx.scaled(arrangement.scale_of(rect) * factor);
        // 一个组件画不出来，只让它这一帧空着，不拖累整张大屏。
        if let Some(tile) = guarded(kind.key(), || tiles::render(&tile_ctx, *kind, scaled)) {
            root = root.child(tile);
        }
    }
    root
}

fn health_label(level: Level) -> String {
    match level {
        Level::Healthy => t!("dash_health_healthy"),
        Level::Warning => t!("dash_health_warning"),
        Level::Critical => t!("dash_health_critical"),
    }
    .into_owned()
}

/// 顶栏：标题和系统信息在左，健康状态和日期时间在右。
fn top_bar(c: &Ctx, rect: Rect, inset: f32, hidden: usize) -> AnyElement {
    let app = c.app;
    let snapshot = &app.snapshot;
    let pal = &c.pal;
    let title = {
        let custom = c.cfg.title.trim();
        let host = snapshot.host.hostname.trim();
        if !custom.is_empty() {
            custom.to_string()
        } else if !host.is_empty() {
            host.to_string()
        } else {
            t!("app_name").into_owned()
        }
    };
    let system = crate::fmt::os_title(&snapshot.os);
    let level = health::overall(&app.alerts());
    let level_color = pal.level(level);

    let left = h_flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(c.px(12.))
        .child(
            div()
                .flex_none()
                .size(c.px(34.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(c.px(10.))
                .bg(pal.cpu.opacity(0.16))
                .child(
                    Icon::new(IconName::Cpu)
                        .with_size(c.px(20.))
                        .text_color(pal.cpu),
                ),
        )
        .child(
            v_flex()
                .min_w_0()
                .gap(c.px(1.))
                .child(c.strong(title, 19., pal.text).truncate())
                .when(!system.trim().is_empty(), |column| {
                    column.child(c.text(system, 11., pal.muted).truncate())
                }),
        );

    let right = h_flex()
        .flex_none()
        .items_center()
        .gap(c.px(18.))
        .when(hidden > 0, |row| {
            row.child(
                h_flex()
                    .items_center()
                    .gap(c.px(5.))
                    .child(
                        Icon::new(IconName::EyeOff)
                            .with_size(c.px(13.))
                            .text_color(pal.muted),
                    )
                    .child(c.text(t!("bs_hidden", n = hidden).into_owned(), 11., pal.muted)),
            )
        })
        .child(
            h_flex()
                .items_center()
                .gap(c.px(6.))
                .px(c.px(10.))
                .py(c.px(4.))
                .rounded_full()
                .bg(level_color.opacity(0.14))
                .child(div().size(c.px(7.)).rounded_full().bg(level_color))
                .child(c.strong(health_label(level), 12., level_color)),
        )
        .when(c.cfg.show_clock, |row| {
            row.child(
                v_flex()
                    .items_end()
                    .gap(c.px(1.))
                    .child(c.number(
                        tiles::clock_text(&c.now, c.cfg.hour24, c.cfg.show_seconds),
                        24.,
                        pal.text,
                    ))
                    .child(c.text(tiles::date_text(&c.now), 11., pal.muted)),
            )
        });

    h_flex()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.w))
        .h(px(rect.h))
        .pl(px(inset))
        .items_center()
        .justify_between()
        .gap(c.px(16.))
        .child(left)
        .child(right)
        .into_any_element()
}

/// 记下窗口现在是不是全屏，以及窗口模式下的位置和大小（全屏时的窗口框不是用户摆的，不记）。
/// 这样用系统自带的按钮进出全屏，下次打开也会是离开时的样子。
pub fn remember(state: &Entity<AppState>, window: &Window, cx: &mut App) {
    let fullscreen = window.is_fullscreen();
    let windowed = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => Some(WindowState {
            x: bounds.origin.x.as_f32(),
            y: bounds.origin.y.as_f32(),
            width: bounds.size.width.as_f32(),
            height: bounds.size.height.as_f32(),
        }),
        _ => None,
    };
    state.update(cx, |s, _| {
        let cfg = &mut s.settings.bigscreen;
        cfg.fullscreen = fullscreen;
        if windowed.is_some() {
            cfg.window = windowed;
        }
    });
}

/// 大屏窗口的内容视图。
pub struct BigScreen {
    state: Entity<AppState>,
    focus: FocusHandle,
    /// 工具条显示到什么时候。
    toolbar_until: Instant,
    /// 「保持唤醒」现在对应的设置值；只在设置变化时才去启停，免得失败了每秒重试。
    awake_applied: bool,
    awake: Option<Awake>,
    _subscriptions: Vec<Subscription>,
}

impl BigScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| {
                this.sync_awake(cx);
                cx.notify();
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                remember(&this.state, window, cx);
            }),
        ];

        // 点红叉关窗口前，把位置存盘。
        let close_state = state.clone();
        window.on_window_should_close(cx, move |window, cx| {
            remember(&close_state, window, cx);
            close_state.read(cx).settings.save();
            true
        });

        // 一秒一跳：时钟的秒针和「工具条该收起了」都靠它。
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut this = Self {
            state,
            focus,
            // 刚打开时让工具条露一会儿脸，用户才知道有它。
            toolbar_until: Instant::now() + TOOLBAR_FOR * 2,
            awake_applied: false,
            awake: None,
            _subscriptions: subscriptions,
        };
        this.sync_awake(cx);
        this
    }

    fn sync_awake(&mut self, cx: &mut Context<Self>) {
        let wanted = self.state.read(cx).settings.bigscreen.keep_awake;
        if wanted != self.awake_applied {
            self.awake_applied = wanted;
            self.awake = if wanted { Awake::start() } else { None };
        }
    }

    fn toolbar_visible(&self) -> bool {
        Instant::now() < self.toolbar_until
    }

    fn edit(&self, cx: &mut Context<Self>, change: impl FnOnce(&mut BigScreenSettings)) {
        self.state.update(cx, |s, cx| {
            s.update_settings(|v| change(&mut v.bigscreen), cx)
        });
    }

    fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = !window.is_fullscreen();
        window.toggle_fullscreen();
        self.edit(cx, |cfg| cfg.fullscreen = target);
    }

    fn step_theme(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.edit(cx, |cfg| cfg.theme = cfg.theme.step(forward));
    }

    fn zoom(&mut self, up: bool, cx: &mut Context<Self>) {
        self.edit(cx, |cfg| {
            let next = if up {
                cfg.scale.saturating_add(SCALE_STEP)
            } else {
                cfg.scale.saturating_sub(SCALE_STEP)
            };
            cfg.scale = next.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
        });
    }

    fn show_settings(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |s, cx| s.set_page(Page::BigScreen, cx));
        crate::workspace::show_main_window(cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        remember(&self.state, window, cx);
        self.state.read(cx).settings.save();
        window.remove_window();
    }

    fn toolbar(&self, cx: &Context<Self>, pal: Palette, fullscreen: bool, theme: String) -> Div {
        let pill = h_flex()
            .items_center()
            .gap(px(2.))
            .p(px(5.))
            .rounded_full()
            .bg(pal.bg[0].opacity(0.95))
            .border_1()
            .border_color(pal.tile_edge)
            .shadow(vec![
                BoxShadow::new(px(0.), px(8.), hsla(0., 0., 0., 0.35)).blur_radius(px(24.)),
            ])
            .child(tool_button(
                pal,
                "bs-prev-theme",
                IconName::ChevronLeft,
                t!("bs_prev_theme").into_owned(),
                cx.listener(|this, _, _, cx| this.step_theme(false, cx)),
            ))
            .child(
                div()
                    .min_w(px(112.))
                    .px(px(6.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(pal.text)
                    .child(theme),
            )
            .child(tool_button(
                pal,
                "bs-next-theme",
                IconName::ChevronRight,
                t!("bs_next_theme").into_owned(),
                cx.listener(|this, _, _, cx| this.step_theme(true, cx)),
            ))
            .child(div().w(px(1.)).h(px(20.)).mx(px(4.)).bg(pal.tile_edge))
            .child(tool_button(
                pal,
                "bs-fullscreen",
                if fullscreen {
                    IconName::Minimize
                } else {
                    IconName::Maximize
                },
                if fullscreen {
                    t!("bs_exit_fullscreen").into_owned()
                } else {
                    t!("bs_fullscreen").into_owned()
                },
                cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx)),
            ))
            .child(tool_button(
                pal,
                "bs-settings",
                IconName::Settings,
                t!("bs_open_settings").into_owned(),
                cx.listener(|this, _, _, cx| this.show_settings(cx)),
            ))
            .child(tool_button(
                pal,
                "bs-close",
                IconName::X,
                t!("bs_close").into_owned(),
                cx.listener(|this, _, window, cx| this.close(window, cx)),
            ));
        h_flex()
            .absolute()
            .bottom(px(26.))
            .left_0()
            .w_full()
            .justify_center()
            .child(pill)
    }
}

fn tool_button(
    pal: Palette,
    id: &'static str,
    icon: IconName,
    tip: String,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let tip = SharedString::from(tip);
    let hover = pal.faint;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(38.))
        .rounded_full()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).with_size(px(19.)).text_color(pal.text))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(on_click)
}

impl Render for BigScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let fullscreen = window.is_fullscreen();
        let toolbar_visible = self.toolbar_visible();
        let state = self.state.read(cx);
        let cfg = &state.settings.bigscreen;
        let pal = cfg.theme.palette();
        let on_mac = cfg!(target_os = "macos");
        // macOS 窗口模式下标题栏是透明的，红绿灯压在顶栏左边，要给它让位。
        let inset = if on_mac && !fullscreen {
            TRAFFIC_LIGHTS
        } else {
            0.
        };
        // 没有顶栏可以给红绿灯让位的时候，整张画面从标题栏下面开始，免得它们压在第一个磁贴上。
        let titlebar = if on_mac && !fullscreen && !cfg.show_header {
            TITLEBAR
        } else {
            0.
        };
        let canvas = Canvas {
            width: size.width.as_f32(),
            height: (size.height.as_f32() - titlebar).max(1.),
            factor: 1.,
        };
        let mono = pal.mono.then(|| cx.theme().mono_font_family.clone());
        let content = guarded("render big screen", || {
            wall(state, cfg, canvas, inset, mono)
        })
        .unwrap_or_else(div);
        let content = if titlebar > 0. {
            div()
                .absolute()
                .top(px(titlebar))
                .left_0()
                .w_full()
                .h(px(canvas.height))
                .child(content)
        } else {
            content
        };
        // 顶栏当标题栏用：窗口模式下拖它就是拖窗口，双击缩放（原生标题栏已经让给我们了）。
        let drag_strip = if cfg.show_header {
            Frame::new(cfg, canvas.width, canvas.height).area.y
        } else {
            TITLEBAR
        };
        let theme = cfg.theme.title();

        let root = div()
            .id("bigscreen")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(pal.bg[0])
            .on_action(cx.listener(|this, _: &ToggleFullscreen, window, cx| {
                this.toggle_fullscreen(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Leave, window, cx| {
                if window.is_fullscreen() {
                    this.toggle_fullscreen(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTheme, _, cx| this.step_theme(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousTheme, _, cx| this.step_theme(false, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom(true, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleHeader, _, cx| {
                this.edit(cx, |cfg| cfg.show_header = !cfg.show_header)
            }))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.show_settings(cx)))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, cx| {
                let was_visible = this.toolbar_visible();
                this.toolbar_until = Instant::now() + TOOLBAR_FOR;
                // 工具条已经在了就不必重画：每秒的节拍会在时间到了之后把它收起来。
                if !was_visible {
                    cx.notify();
                }
            }))
            .child(content);
        let root = if on_mac && !fullscreen {
            root.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(drag_strip))
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count >= 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
        } else {
            root
        };
        if toolbar_visible {
            root.child(self.toolbar(cx, pal, fullscreen, theme))
        } else {
            root
        }
    }
}
