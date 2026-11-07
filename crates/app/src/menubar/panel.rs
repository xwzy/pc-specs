//! 点击菜单栏项后弹出的监控面板：贴着被点的那一项、落在它所在的显示器上，只展示这一个指标。
//! 内容放不下时不滚动，而是把鼠标悬停的那一块展开成旁边的二级页面。
use super::{
    Anchor, color,
    config::Metric,
    content::{self, Block, Flyout, PAD, Side, WIDTH},
    icon, metric_page,
};
use crate::state::{AppState, Page};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, DisplayId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, actions, div, point,
    prelude::FluentBuilder as _, px, relative, size,
};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::time::Duration;

actions!(monitor_panel, [Dismiss]);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Dismiss, Some("MonitorPanel"))]);
}

const HEADER: f32 = 60.;
const FOOTER: f32 = 44.;
/// 上下各 1px 的边框。
const BORDER: f32 = 2.;
const RADIUS: f32 = 12.;
/// 面板和菜单栏之间留的缝。
const BAR_GAP: f32 = 2.;
/// 面板和屏幕边缘至少隔开的距离。
const MARGIN: f32 = 8.;
/// 面板和二级页面之间的缝。
const FLYOUT_GAP: f32 = 4.;
/// 屏幕太矮时，进程排行最少保留的行数。
const MIN_ROWS: usize = 3;
const OPEN_DELAY: Duration = Duration::from_millis(140);
const CLOSE_DELAY: Duration = Duration::from_millis(220);
/// 面板或二级页面失去焦点后，等这么久再看用户是不是真的离开了（焦点从一个窗口交到另一个窗口有个先后）。
const LEAVE_DELAY: Duration = Duration::from_millis(120);
/// 面板因为用户点击菜单栏项而失去焦点关闭后，这段时间里的点击算作「收起」，而不是重新打开。
const TOGGLE_WINDOW: Duration = Duration::from_millis(500);

/// 显示器上可以放窗口的区域（不含菜单栏和 Dock），单位 pt，原点是该显示器的左上角。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Area {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Area {
    fn of(bounds: Bounds<Pixels>) -> Self {
        Self {
            left: bounds.left().as_f32(),
            top: bounds.top().as_f32(),
            right: bounds.right().as_f32(),
            bottom: bounds.bottom().as_f32(),
        }
    }

    fn height(self) -> f32 {
        self.bottom - self.top
    }
}

/// 面板左上角：左边缘对齐被点的菜单栏项（像下拉菜单一样），贴在菜单栏下沿，靠近屏幕右边时整体往里收。
fn place(anchor: Option<Anchor>, area: Area, width: f32, height: f32) -> (f32, f32) {
    let max_x = (area.right - width - MARGIN).max(area.left + MARGIN);
    let (x, top) = match anchor {
        Some(a) => (
            a.left.clamp(area.left + MARGIN, max_x),
            a.bottom.max(area.top) + BAR_GAP,
        ),
        None => (max_x, area.top + BAR_GAP),
    };
    let max_y = (area.bottom - height - MARGIN).max(area.top);
    (x, top.min(max_y))
}

/// 二级页面放在面板的哪一侧：右边放得下就放右边，否则放左边，两边都放不下取空间大的一边。
fn flyout_side(panel_x: f32, area: Area, width: f32, flyout_width: f32) -> Side {
    let right = area.right - MARGIN - (panel_x + width + FLYOUT_GAP);
    let left = panel_x - FLYOUT_GAP - (area.left + MARGIN);
    if right >= flyout_width || (left < flyout_width && right >= left) {
        Side::Right
    } else {
        Side::Left
    }
}

/// 二级页面的左上角：和面板顶部对齐，贴着面板的一侧。
fn place_flyout(
    panel: (f32, f32),
    side: Side,
    area: Area,
    width: f32,
    flyout: (f32, f32),
) -> (f32, f32) {
    let x = match side {
        Side::Right => panel.0 + width + FLYOUT_GAP,
        Side::Left => panel.0 - FLYOUT_GAP - flyout.0,
    };
    let x = x.clamp(area.left, (area.right - flyout.0).max(area.left));
    let y = panel.1.min((area.bottom - MARGIN - flyout.1).max(area.top));
    (x, y)
}

fn panel_height(blocks: &[Block]) -> f32 {
    BORDER + HEADER + blocks.iter().map(|b| b.height).sum::<f32>() + FOOTER
}

fn popup(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    focus: bool,
    display: Option<DisplayId>,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(x), px(y)),
            size(px(width), px(height)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        display_id: display,
        ..Default::default()
    }
}

pub fn show(state: Entity<AppState>, metric: Metric, anchor: Option<Anchor>, cx: &mut App) {
    if !crate::workspace::screens_available(cx) {
        tracing::warn!("no display available, not opening the monitor panel");
        return;
    }
    let (old, same, just_closed) = {
        let s = state.read(cx);
        (
            s.detail_window,
            s.detail_metric == Some(metric),
            s.detail_closed
                .is_some_and(|(m, at)| m == metric && at.elapsed() < TOGGLE_WINDOW),
        )
    };
    if let Some(handle) = old {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        if same {
            return;
        }
    } else if just_closed {
        // 点击菜单栏项会先让面板失去焦点而关闭，随后才收到这一下点击：它的意思是「收起」。
        state.update(cx, |s, _| s.detail_closed = None);
        return;
    }

    // 弹在被点击的那一块显示器上；拿不到锚点（比如没有菜单栏的平台）就用主显示器。
    let display = anchor
        .and_then(|a| cx.find_display(DisplayId::new(a.display)))
        .or_else(|| cx.primary_display());
    let area = display.as_ref().map_or(
        Area {
            left: 0.,
            top: 24.,
            right: 1280.,
            bottom: 800.,
        },
        |d| Area::of(d.visible_bounds()),
    );
    let top = anchor.map_or(area.top, |a| a.bottom.max(area.top)) + BAR_GAP;
    let (x, _) = place(anchor, area, WIDTH, 0.);
    let side = flyout_side(x, area, WIDTH, WIDTH);
    let (rows, height) = {
        let s = state.read(cx);
        let mut rows = s.settings.menu_bar.process_rows;
        loop {
            let height = panel_height(&content::blocks(s, metric, side, rows, cx));
            if height <= area.bottom - MARGIN - top || rows <= MIN_ROWS {
                break (rows, height);
            }
            rows -= 1;
        }
    };
    let (x, y) = place(anchor, area, WIDTH, height);
    let options = popup(x, y, WIDTH, height, true, display.map(|d| d.id()));
    let geometry = Geometry {
        origin: (x, y),
        area,
        side,
        rows,
    };
    let display_id = options.display_id;
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            MetricPanel::new(
                state.clone(),
                metric,
                geometry,
                display_id,
                height,
                window,
                cx,
            )
        })
    }) {
        Ok((handle, _)) => {
            state.update(cx, |s, cx| {
                s.detail_window = Some(handle);
                s.detail_metric = Some(metric);
                s.detail_closed = None;
                s.highlight_metric(Some(metric));
                s.sync_process_feed(cx);
                s.refresh_live(cx);
                cx.notify();
            });
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(error) => tracing::error!("failed to open monitor detail: {error}"),
    }
}

/// 面板打开时就定下的几何信息。
#[derive(Clone, Copy)]
struct Geometry {
    origin: (f32, f32),
    area: Area,
    side: Side,
    rows: usize,
}

struct OpenFlyout {
    kind: Flyout,
    handle: AnyWindowHandle,
}

struct MetricPanel {
    state: Entity<AppState>,
    metric: Metric,
    geometry: Geometry,
    display: Option<DisplayId>,
    window: AnyWindowHandle,
    /// 窗口当前的高度，内容高度变了（比如多了一块卷）时据此调整窗口。
    height: f32,
    pinned: bool,
    focus: FocusHandle,
    flyout: Option<OpenFlyout>,
    over_trigger: Option<Flyout>,
    over_flyout: bool,
    /// 等待展开 / 收起二级页面的计时，被新的悬停取代时自动取消。
    pending: Option<Task<()>>,
    /// 失去焦点后等着确认用户是否离开的计时。
    leaving: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MetricPanel {
    fn new(
        state: Entity<AppState>,
        metric: Metric,
        geometry: Geometry,
        display: Option<DisplayId>,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            window.observe_window_appearance({
                let state = state.clone();
                move |window, cx| {
                    if state.read(cx).settings.theme == crate::settings::ThemePref::System {
                        gpui_kit::component::Theme::sync_system_appearance(Some(window), cx);
                    }
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.leave_check(cx);
                }
            }),
            cx.on_release(|this, cx| {
                if let Some(open) = this.flyout.take() {
                    let _ = open
                        .handle
                        .update(cx, |_, window, _| window.remove_window());
                }
            }),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            state,
            metric,
            geometry,
            display,
            window: window.window_handle(),
            height,
            pinned: false,
            focus,
            flyout: None,
            over_trigger: None,
            over_flyout: false,
            pending: None,
            leaving: None,
            _subscriptions: subscriptions,
        }
    }

    /// 面板或二级页面失去了焦点：稍后两个窗口都不在前台才关掉，点进二级页面不算离开。
    fn leave_check(&mut self, cx: &mut Context<Self>) {
        if self.pinned {
            return;
        }
        self.leaving = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let active = |handle: AnyWindowHandle, cx: &mut App| {
                    handle
                        .update(cx, |_, window, _| window.is_window_active())
                        .unwrap_or(false)
                };
                let flyout = this.flyout.as_ref().map(|open| open.handle);
                if !this.pinned
                    && !active(this.window, cx)
                    && !flyout.is_some_and(|handle| active(handle, cx))
                {
                    let _ = this
                        .window
                        .update(cx, |_, window, _| window.remove_window());
                }
            });
        }));
    }

    fn hover_trigger(&mut self, kind: Flyout, hovered: bool, cx: &mut Context<Self>) {
        let delay = if hovered {
            self.over_trigger = Some(kind);
            // 已经有二级页面时，换一块悬停就直接切换，不用再等。
            if self.flyout.is_some() {
                Duration::ZERO
            } else {
                OPEN_DELAY
            }
        } else {
            if self.over_trigger == Some(kind) {
                self.over_trigger = None;
            }
            CLOSE_DELAY
        };
        self.settle(delay, cx);
    }

    fn hover_flyout(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.over_flyout = hovered;
        self.settle(if hovered { Duration::ZERO } else { CLOSE_DELAY }, cx);
    }

    /// 过一会儿再根据鼠标当前的位置决定二级页面的去留，鼠标从面板移到二级页面的途中不会闪一下。
    fn settle(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.pending = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let _ = this.update(cx, |this, cx| this.reconcile(cx));
        }));
    }

    fn reconcile(&mut self, cx: &mut Context<Self>) {
        let wanted = if self.over_flyout {
            self.flyout.as_ref().map(|open| open.kind)
        } else {
            self.over_trigger
        };
        if wanted == self.flyout.as_ref().map(|open| open.kind) {
            return;
        }
        self.close_flyout(cx);
        if let Some(kind) = wanted {
            self.open_flyout(kind, cx);
        }
    }

    fn close_flyout(&mut self, cx: &mut Context<Self>) {
        self.over_flyout = false;
        if let Some(open) = self.flyout.take() {
            // 点进过二级页面的话焦点在它身上：收起前还给面板，之后点到别处才能照常收起面板。
            let focused = open
                .handle
                .update(cx, |_, window, _| window.is_window_active())
                .unwrap_or(false);
            if focused {
                let _ = self
                    .window
                    .update(cx, |_, window, _| window.activate_window());
            }
            let _ = open
                .handle
                .update(cx, |_, window, _| window.remove_window());
        }
        cx.notify();
    }

    fn open_flyout(&mut self, kind: Flyout, cx: &mut Context<Self>) {
        let Geometry {
            origin, area, side, ..
        } = self.geometry;
        let max_height = area.height() - 2. * MARGIN;
        let (width, height, _) =
            content::flyout(self.state.read(cx), self.metric, kind, max_height, cx);
        let (x, y) = place_flyout(origin, side, area, WIDTH, (width, height));
        let options = popup(x, y, width, height, false, self.display);
        let (state, metric) = (self.state.clone(), self.metric);
        let panel = cx.entity().downgrade();
        match gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let owner = panel.clone();
                FlyoutView {
                    _subscriptions: vec![
                        cx.observe(&state, |_, _, cx| cx.notify()),
                        // 用户点进二级页面再点到别处时，也要由面板确认是不是该收起了。
                        cx.observe_window_activation(window, move |_, window, cx| {
                            if !window.is_window_active() {
                                let _ = owner.update(cx, |panel, cx| panel.leave_check(cx));
                            }
                        }),
                    ],
                    state,
                    panel,
                    metric,
                    kind,
                    max_height,
                    height,
                }
            })
        }) {
            Ok((handle, _)) => {
                self.flyout = Some(OpenFlyout { kind, handle });
                cx.notify();
            }
            Err(error) => tracing::error!("failed to open monitor flyout: {error}"),
        }
    }

    fn header(&self, subtitle: String, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = color(self.metric, cx);
        h_flex()
            .h(px(HEADER))
            .flex_shrink_0()
            .px(px(PAD))
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .size(px(34.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .bg(accent.opacity(0.16))
                    .child(
                        Icon::new(icon(self.metric))
                            .size(px(18.))
                            .text_color(accent),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(relative(1.3))
                            .font_semibold()
                            .child(self.metric.title()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(relative(1.3))
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .child(
                Button::new("pin")
                    .ghost()
                    .small()
                    .icon(if self.pinned {
                        IconName::PinOff
                    } else {
                        IconName::Pin
                    })
                    .when(self.pinned, |b| b.primary())
                    .tooltip(t!("bar_pin").into_owned())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pinned = !this.pinned;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("close")
                    .ghost()
                    .small()
                    .icon(IconName::X)
                    .on_click(|_, window, _| window.remove_window()),
            )
    }

    fn footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let metric = self.metric;
        let (open_state, config_state) = (self.state.clone(), self.state.clone());
        h_flex()
            .h(px(FOOTER))
            .flex_shrink_0()
            .px(px(PAD - 6.))
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("open-details")
                    .ghost()
                    .small()
                    .icon(IconName::ExternalLink)
                    .label(t!("bar_open_detail").into_owned())
                    .on_click(move |_, window, cx| {
                        window.remove_window();
                        open_state.update(cx, |s, cx| s.set_page(metric_page(metric), cx));
                        crate::workspace::show_main_window(cx);
                    }),
            )
            .child(
                Button::new("configure")
                    .ghost()
                    .small()
                    .icon(IconName::Settings2)
                    .label(t!("bar_configure").into_owned())
                    .on_click(move |_, window, cx| {
                        window.remove_window();
                        config_state.update(cx, |s, cx| s.set_page(Page::MenuBar, cx));
                        crate::workspace::show_main_window(cx);
                    }),
            )
    }
}

impl Render for MetricPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (blocks, subtitle) = {
            let s = self.state.read(cx);
            // 绘制由系统回调直接驱动，这里 panic 会让整个进程 abort：画不出来就先空着。
            let blocks = guarded("panel content", || {
                content::blocks(s, self.metric, self.geometry.side, self.geometry.rows, cx)
            })
            .unwrap_or_default();
            (blocks, content::subtitle(s, self.metric))
        };
        let height = panel_height(&blocks);
        if (height - self.height).abs() >= 0.5 {
            self.height = height;
            window.defer(cx, move |window, _| {
                window.resize(size(px(WIDTH), px(height)));
            });
        }
        let theme = cx.theme();
        let hover = theme.foreground.opacity(0.05);
        let open = self.flyout.as_ref().map(|open| open.kind);
        let rows = blocks.into_iter().enumerate().map(|(index, block)| {
            let row = div()
                .id(("block", index))
                .w_full()
                .flex_shrink_0()
                .px(px(PAD))
                .child(block.element);
            match block.trigger {
                Some(kind) => row
                    .when(open == Some(kind), |row| row.bg(hover))
                    .hover(move |style| style.bg(hover))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hover_trigger(kind, *hovered, cx)
                    }))
                    .into_any_element(),
                None => row.into_any_element(),
            }
        });
        v_flex()
            .id("monitor-detail")
            .key_context("MonitorPanel")
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .rounded(px(RADIUS))
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .on_action(cx.listener(|_, _: &Dismiss, window, _| window.remove_window()))
            .child(self.header(subtitle, cx))
            .child(v_flex().flex_1().min_h_0().children(rows))
            .child(self.footer(cx))
    }
}

/// 二级页面：只读，鼠标移进来时告诉面板「别收」。
struct FlyoutView {
    state: Entity<AppState>,
    panel: WeakEntity<MetricPanel>,
    metric: Metric,
    kind: Flyout,
    max_height: f32,
    height: f32,
    _subscriptions: Vec<Subscription>,
}

impl Render for FlyoutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, height, body) = guarded("flyout content", || {
            content::flyout(
                self.state.read(cx),
                self.metric,
                self.kind,
                self.max_height,
                cx,
            )
        })
        .unwrap_or_else(|| (0., self.height, div().into_any_element()));
        if (height - self.height).abs() >= 0.5 {
            self.height = height;
            window.defer(cx, move |window, _| {
                window.resize(size(px(WIDTH), px(height)));
            });
        }
        let theme = cx.theme();
        div()
            .id("monitor-flyout")
            .size_full()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .rounded(px(RADIUS))
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                let hovered = *hovered;
                let _ = this
                    .panel
                    .update(cx, |panel, cx| panel.hover_flyout(hovered, cx));
            }))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Area = Area {
        left: 0.,
        top: 33.,
        right: 1512.,
        bottom: 944.,
    };

    fn anchor(left: f32, bottom: f32) -> Anchor {
        Anchor {
            display: 1,
            left,
            right: left + 48.,
            bottom,
        }
    }

    #[test]
    fn panel_hangs_from_the_clicked_item() {
        let (x, y) = place(Some(anchor(600., 33.)), MAIN, WIDTH, 560.);
        assert_eq!((x, y), (600., 33. + BAR_GAP));
    }

    #[test]
    fn panel_is_pulled_back_inside_near_the_screen_edge() {
        let (x, _) = place(Some(anchor(1480., 33.)), MAIN, WIDTH, 560.);
        assert_eq!(x, MAIN.right - WIDTH - MARGIN);
        let (x, _) = place(Some(anchor(-30., 33.)), MAIN, WIDTH, 560.);
        assert_eq!(x, MARGIN);
    }

    #[test]
    fn follows_the_menu_bar_of_any_display() {
        // 副屏菜单栏矮一点，面板就跟着往上，不会按主屏的菜单栏高度去放。
        let dell = Area {
            left: 0.,
            top: 30.,
            right: 1920.,
            bottom: 1080.,
        };
        let (_, y) = place(Some(anchor(1500., 30.)), dell, WIDTH, 560.);
        assert_eq!(y, 30. + BAR_GAP);
        // 菜单栏高度没报准（比可用区域还靠上）时，也不盖住菜单栏。
        let (_, y) = place(Some(anchor(1500., 0.)), dell, WIDTH, 560.);
        assert_eq!(y, 30. + BAR_GAP);
    }

    #[test]
    fn tall_content_never_leaves_the_screen() {
        let small = Area {
            left: 0.,
            top: 24.,
            right: 1280.,
            bottom: 700.,
        };
        // 放得下：贴着菜单栏。
        let (_, y) = place(Some(anchor(800., 24.)), small, WIDTH, 600.);
        assert_eq!(y, 24. + BAR_GAP);
        // 比屏幕还高：顶到可用区域的上沿，不盖住菜单栏。
        let (_, y) = place(Some(anchor(800., 24.)), small, WIDTH, 800.);
        assert_eq!(y, small.top);
        // 锚点本身很靠下（比如 Dock 在顶部的奇怪布局）：整体上移，底边留出边距。
        let (_, y) = place(Some(anchor(800., 400.)), small, WIDTH, 400.);
        assert_eq!(y, small.bottom - MARGIN - 400.);
    }

    #[test]
    fn flyout_opens_towards_the_roomier_side() {
        // 面板靠右：右边放不下，开在左边，并且紧挨着面板。
        let panel = (MAIN.right - WIDTH - MARGIN, 35.);
        assert_eq!(flyout_side(panel.0, MAIN, WIDTH, WIDTH), Side::Left);
        let (x, y) = place_flyout(panel, Side::Left, MAIN, WIDTH, (WIDTH, 500.));
        assert_eq!((x, y), (panel.0 - FLYOUT_GAP - WIDTH, 35.));
        // 面板靠左：开在右边。
        assert_eq!(flyout_side(20., MAIN, WIDTH, WIDTH), Side::Right);
        let (x, _) = place_flyout((20., 35.), Side::Right, MAIN, WIDTH, (WIDTH, 500.));
        assert_eq!(x, 20. + WIDTH + FLYOUT_GAP);
        // 两边都放不下（很窄的屏幕）：取空间大的一边，且不超出屏幕。
        let narrow = Area {
            left: 0.,
            top: 24.,
            right: 500.,
            bottom: 800.,
        };
        let side = flyout_side(150., narrow, WIDTH, WIDTH);
        let (x, _) = place_flyout((150., 24.), side, narrow, WIDTH, (WIDTH, 400.));
        assert!(x >= narrow.left && x + WIDTH <= narrow.right);
    }

    #[test]
    fn flyout_stays_above_the_bottom_edge() {
        let (_, y) = place_flyout((900., 600.), Side::Left, MAIN, WIDTH, (WIDTH, 500.));
        assert_eq!(y, MAIN.bottom - MARGIN - 500.);
        // 放得下就和面板顶部对齐。
        let (_, y) = place_flyout((900., 40.), Side::Left, MAIN, WIDTH, (WIDTH, 500.));
        assert_eq!(y, 40.);
    }
}
