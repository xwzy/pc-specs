//! 主窗口：标题栏 + 侧边栏 + 当前页面 + 状态栏，以及主窗口的开 / 关 / 退出管理。

use crate::health::{self, Level};
use crate::pages::{self, Pages};
use crate::settings::{ThemePref, WindowState};
use crate::state::{AppState, Page};
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Icon, Selectable as _, Side, Sizable as _, StyledExt as _,
    Theme, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    separator::Separator,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarHeader, SidebarItem, SidebarMenu, SidebarMenuItem,
        SidebarToggleButton,
    },
    status_bar::StatusBar,
    v_flex,
};
use gpui_kit::{Action, App, KeyBinding};
use gpui_kit::{
    AppContext as _, Bounds, Context, ElementId, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, WindowBounds, WindowOptions, actions, div, point,
    prelude::FluentBuilder as _, px, size,
};
#[cfg(target_os = "macos")]
use gpui_kit::{Menu, MenuItem};
use pc_specs_core::util::guarded;
use rust_i18n::t;

actions!(
    pc_specs,
    [
        Quit,
        Refresh,
        OpenSettings,
        ToggleSidebar,
        CloseWindow,
        MinimizeWindow,
        ZoomWindow
    ]
);

const APP_ID: &str = "com.xwzy.pc-specs";
// 高度留够：侧栏的全部入口要在默认窗口里不用滚动就能看到。
const DEFAULT_SIZE: (f32, f32) = (1180.0, 800.0);
const MIN_SIZE: (f32, f32) = (920.0, 600.0);
/// 侧栏展开 / 收起（图标栏）时的宽度，页面据此算出自己的可用宽度。
const SIDEBAR_WIDTH: f32 = 224.0;
const SIDEBAR_COLLAPSED_WIDTH: f32 = 48.0;
/// 页面左右各 24px 的内边距。
const PAGE_PADDING: f32 = 48.0;

pub fn init(cx: &mut App) {
    crate::menubar::panel::init(cx);
    // 主修饰键：macOS 用 Cmd，其他平台用 Ctrl。
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-r", Refresh, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, Some("Workspace")),
    ]);
    on_action(cx, |_: &Quit, cx| quit(cx));
    on_action(cx, |_: &Refresh, cx| {
        let state = AppState::global(cx);
        state.update(cx, |s, cx| s.refresh(cx));
    });
    on_action(cx, |_: &OpenSettings, cx| {
        let state = AppState::global(cx);
        state.update(cx, |s, cx| s.set_page(Page::Settings, cx));
        show_main_window(cx);
    });
    on_action(cx, |_: &CloseWindow, cx| {
        on_active_window(cx, |window| window.remove_window())
    });
    on_action(cx, |_: &MinimizeWindow, cx| {
        on_active_window(cx, |window| window.minimize_window())
    });
    on_action(cx, |_: &ZoomWindow, cx| {
        on_active_window(cx, |window| window.zoom_window())
    });
}

/// 菜单和快捷键触发的动作由系统的事件循环直接调用，里面 panic 会让整个进程 abort。
/// 出了问题只丢掉这一次操作，应用照常运行。
pub fn on_action<A: Action>(cx: &mut App, handler: impl Fn(&A, &mut App) + 'static) {
    cx.on_action(move |action: &A, cx| {
        guarded("menu action", || handler(action, cx));
    });
}

/// 对当前最前面的窗口做点什么（关闭 / 最小化 / 缩放）。
fn on_active_window(cx: &mut App, f: impl FnOnce(&mut Window)) {
    if let Some(handle) = cx.active_window() {
        let _ = handle.update(cx, |_, window, _| f(window));
    }
}

/// macOS 应用菜单：设置、退出，以及窗口的最小化 / 缩放 / 关闭（⌘M / ⌘W）。
/// 菜单文字跟着界面语言走，所以语言变了要重新装一遍。
pub fn install_menus(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.set_menus([
        Menu::new(t!("app_name").into_owned()).items([
            MenuItem::action(t!("menu_settings").into_owned(), OpenSettings),
            MenuItem::action(t!("bs_open").into_owned(), crate::bigscreen::OpenBigScreen),
            MenuItem::separator(),
            MenuItem::action(t!("tray_quit").into_owned(), Quit),
        ]),
        Menu::new(t!("menu_window").into_owned()).items([
            MenuItem::action(t!("menu_minimize").into_owned(), MinimizeWindow),
            MenuItem::action(t!("menu_zoom").into_owned(), ZoomWindow),
            MenuItem::action(t!("menu_close").into_owned(), CloseWindow),
        ]),
    ]);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

pub struct Workspace {
    state: Entity<AppState>,
    pages: Pages,
    sidebar_collapsed: bool,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// 侧栏分组。比自带的 `SidebarGroup` 紧凑：小号标题、收紧的行距，标题也可以省略，
/// 这样默认窗口高度下全部入口都能直接看到，不用在侧栏里再滚动。
#[derive(Clone)]
struct NavGroup {
    label: Option<SharedString>,
    menu: SidebarMenu,
    collapsed: bool,
}

impl NavGroup {
    fn new(label: Option<String>, menu: SidebarMenu) -> Self {
        Self {
            label: label.map(SharedString::from),
            menu,
            collapsed: false,
        }
    }
}

impl Collapsible for NavGroup {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl SidebarItem for NavGroup {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let collapsed = self.collapsed;
        let labeled = self.label.is_some();
        let label_color = cx.theme().sidebar_foreground.opacity(0.7);
        let divider = cx.theme().sidebar_border;
        v_flex()
            // 展开时靠标题隔开各组；收成图标栏后标题没了，改用一条细线。
            .when(labeled && !collapsed, |this| this.pt_3())
            .when(labeled && collapsed, |this| {
                this.mt_2().pt_2().border_t_1().border_color(divider)
            })
            .when_some(self.label.filter(|_| !collapsed), |this, label| {
                this.child(
                    h_flex()
                        .h_6()
                        .px_2()
                        .text_xs()
                        .text_color(label_color)
                        .child(label),
                )
            })
            .child(self.menu.collapsed(collapsed).render(id, window, cx))
    }
}

impl Workspace {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance_state = state.clone();
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            // 系统明暗切换时，「跟随系统」模式要同步主题。
            window.observe_window_appearance(move |window, cx| {
                if appearance_state.read(cx).settings.theme == ThemePref::System {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
        ];

        // 关闭窗口时记住位置与大小。
        let close_state = state.clone();
        window.on_window_should_close(cx, move |window, cx| {
            remember_bounds(&close_state, window, cx);
            true
        });

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let pages = Pages::new(&state, window, cx);
        Self {
            state,
            pages,
            sidebar_collapsed: false,
            focus_handle,
            _subscriptions: subscriptions,
        }
    }

    fn nav_item(&self, page: Page, current: Page) -> SidebarMenuItem {
        let state = self.state.clone();
        SidebarMenuItem::new(page.title())
            .icon(page.icon())
            .active(page == current)
            .on_click(move |_, _, cx| {
                state.update(cx, |s, cx| s.set_page(page, cx));
            })
    }

    fn render_sidebar(&self, current: Page, cx: &App) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let (accent, on_accent) = (cx.theme().primary, cx.theme().primary_foreground);
        let group = |label: Option<String>, pages: &[Page]| {
            NavGroup::new(
                label,
                SidebarMenu::new()
                    .gap_0p5()
                    .children(pages.iter().map(|page| self.nav_item(*page, current))),
            )
        };
        Sidebar::new("main-sidebar")
            .side(Side::Left)
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .w(px(224.))
            .gap_0()
            .header(
                SidebarHeader::new()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size_8()
                            .rounded_lg()
                            .bg(accent)
                            .text_color(on_accent)
                            .child(Icon::new(IconName::Cpu)),
                    )
                    .when(!collapsed, |this| {
                        this.child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_sm()
                                .font_semibold()
                                .child(t!("app_name").into_owned()),
                        )
                    }),
            )
            // 第一组是日常看数据的入口，不加标题。
            .child(group(
                None,
                &[Page::Dashboard, Page::Monitor, Page::Processes],
            ))
            // 顶栏监控与监控大屏是「把监控放到桌面上」的设置页，和看数据的页面分开。
            .child(group(
                Some(t!("nav_group_desktop").into_owned()),
                &[Page::MenuBar, Page::BigScreen],
            ))
            .child(group(
                Some(t!("nav_group_hardware").into_owned()),
                &Page::HARDWARE,
            ))
            .child(group(
                Some(t!("nav_group_system").into_owned()),
                &[Page::Os, Page::DevEnv],
            ))
            .child(group(
                Some(t!("nav_group_tools").into_owned()),
                // 没有「应用」可管的系统（Linux）不摆这一页。
                if pc_specs_core::apps::SUPPORTED {
                    &[Page::Rules, Page::Startup, Page::Apps][..]
                } else {
                    &[Page::Rules, Page::Startup][..]
                },
            ))
    }

    fn render_title_bar(
        &self,
        current: Page,
        refreshing: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dark = cx.theme().mode.is_dark();
        // 按钮上画的是「点了会变成什么」，提示文字也说清楚。
        let theme_hint = if dark {
            t!("theme_switch_light").into_owned()
        } else {
            t!("theme_switch_dark").into_owned()
        };
        let state = self.state.clone();
        let refresh_state = state.clone();
        let export_state = state.clone();
        let settings_state = state.clone();
        TitleBar::new()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        SidebarToggleButton::new()
                            .side(Side::Left)
                            .collapsed(self.sidebar_collapsed)
                            .accessibility_label(t!("nav_toggle_sidebar").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_collapsed = !this.sidebar_collapsed;
                                cx.notify();
                            })),
                    )
                    .child(Separator::vertical().h_4())
                    .child(div().text_sm().font_semibold().child(current.title())),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .pr_2()
                    .child(
                        Button::new("titlebar-export")
                            .ghost()
                            .small()
                            .icon(Page::Export.icon())
                            .label(Page::Export.title())
                            .selected(current == Page::Export)
                            .on_click(move |_, _, cx| {
                                export_state.update(cx, |s, cx| s.set_page(Page::Export, cx));
                            }),
                    )
                    .child(
                        Button::new("titlebar-refresh")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .loading(refreshing)
                            .accessibility_label(t!("common_refresh").into_owned())
                            .tooltip_with_action(t!("common_refresh").into_owned(), &Refresh, None)
                            .on_click(move |_, _, cx| {
                                refresh_state.update(cx, |s, cx| s.refresh(cx));
                            }),
                    )
                    .child(
                        Button::new("titlebar-theme")
                            .ghost()
                            .small()
                            .icon(if dark { IconName::Sun } else { IconName::Moon })
                            .accessibility_label(theme_hint.clone())
                            .tooltip(theme_hint)
                            .on_click(move |_, _, cx| {
                                state.update(cx, |s, cx| {
                                    s.update_settings(
                                        |settings| {
                                            settings.theme = if dark {
                                                ThemePref::Light
                                            } else {
                                                ThemePref::Dark
                                            }
                                        },
                                        cx,
                                    )
                                });
                            }),
                    )
                    .child(
                        Button::new("titlebar-settings")
                            .ghost()
                            .small()
                            .icon(Page::Settings.icon())
                            .selected(current == Page::Settings)
                            .accessibility_label(Page::Settings.title())
                            .tooltip_with_action(Page::Settings.title(), &OpenSettings, None)
                            .on_click(move |_, _, cx| {
                                settings_state.update(cx, |s, cx| s.set_page(Page::Settings, cx));
                            }),
                    ),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let settings = &s.settings;
        let alerts = s.alerts();
        let level = health::overall(&alerts);
        let health_text = match level {
            Level::Healthy => t!("dash_health_healthy"),
            Level::Warning => t!("dash_health_warning"),
            Level::Critical => t!("dash_health_critical"),
        }
        .into_owned();

        // 状态栏上的每一项都能点：直接跳到对应页面看详情。
        let (foreground, hover_bg, radius) = (
            cx.theme().foreground,
            cx.theme().list_hover,
            cx.theme().radius,
        );
        let state = self.state.clone();
        let clickable = |id: &'static str, target: Page| {
            let state = state.clone();
            h_flex()
                .id(id)
                .items_center()
                .gap_1()
                .px_1p5()
                .rounded(radius)
                .cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| s.set_page(target, cx));
                })
        };
        let metric = |id: &'static str, label: String, value: String, target: Page| {
            clickable(id, target)
                .child(div().child(label))
                .child(div().text_color(foreground).child(value))
        };

        let mut left = h_flex().gap_1().items_center();
        match &s.latest {
            Some(tick) => {
                let mem = fmt::ratio(tick.mem_used_bytes, tick.mem_total_bytes);
                left = left
                    .child(metric(
                        "status-cpu",
                        t!("nav_cpu").into_owned(),
                        fmt::percent(tick.cpu_overall),
                        Page::Cpu,
                    ))
                    .child(metric(
                        "status-memory",
                        t!("dash_card_memory").into_owned(),
                        fmt::percent(mem),
                        Page::Memory,
                    ))
                    .child(metric(
                        "status-network",
                        t!("dash_card_network").into_owned(),
                        format!(
                            "↓{} ↑{}",
                            settings.rate(tick.net_rx_bps),
                            settings.rate(tick.net_tx_bps)
                        ),
                        Page::Network,
                    ));
                if let Some(temp) = tick.max_temperature() {
                    left = left.child(metric(
                        "status-temp",
                        t!("dash_card_temp").into_owned(),
                        settings.temp(temp),
                        Page::Sensors,
                    ));
                }
            }
            None => {
                left = left.child(div().px_1p5().child(t!("common_loading").into_owned()));
            }
        }

        StatusBar::new()
            .left(left)
            .right(
                clickable("status-health", Page::Dashboard)
                    .child(ui::level_tag(level, health_text).small()),
            )
            .when(s.paused, |bar| {
                bar.right(ui::chip(t!("monitor_paused").into_owned()))
            })
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (current, refreshing) = {
            let s = self.state.read(cx);
            (s.page, s.refreshing)
        };
        // 页面按自己的可用宽度决定网格列数，窄了就少放几列。
        let sidebar_width = if self.sidebar_collapsed {
            SIDEBAR_COLLAPSED_WIDTH
        } else {
            SIDEBAR_WIDTH
        };
        let content_width = window.viewport_size().width.as_f32() - sidebar_width - PAGE_PADDING;
        let content = pages::render(current, &self.state, &self.pages, content_width, cx);

        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
            .child(self.render_title_bar(current, refreshing, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_sidebar(current, cx))
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
            .child(self.render_status_bar(cx))
    }
}

// ---- 窗口管理 ---------------------------------------------------------------

fn window_alive(cx: &App, handle: gpui_kit::AnyWindowHandle) -> bool {
    cx.windows().contains(&handle)
}

fn remember_bounds(state: &Entity<AppState>, window: &Window, cx: &mut App) {
    let bounds = match window.window_bounds() {
        WindowBounds::Windowed(b) | WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => b,
    };
    let saved = WindowState {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
    };
    state.update(cx, |s, _| s.settings.window = Some(saved));
}

/// 上次的窗口位置如果还在某个显示器内就复用，否则居中。
fn initial_bounds(state: &Entity<AppState>, cx: &App) -> WindowBounds {
    let saved = state.read(cx).settings.window;
    if let Some(w) = saved {
        let bounds = Bounds::new(point(px(w.x), px(w.y)), size(px(w.width), px(w.height)));
        let visible = cx.displays().iter().any(|d| {
            let area = d.bounds();
            area.contains(&bounds.origin) && area.contains(&bounds.bottom_right())
        });
        if visible && w.width >= MIN_SIZE.0 && w.height >= MIN_SIZE.1 {
            return WindowBounds::Windowed(bounds);
        }
    }
    // 默认尺寸不能超过屏幕（小屏笔记本上 760 高可能放不下）。
    let (width, height) = match cx.primary_display() {
        Some(display) => {
            let area = display.bounds().size;
            (
                DEFAULT_SIZE.0.min(area.width.as_f32() - 80.0),
                DEFAULT_SIZE.1.min(area.height.as_f32() - 120.0),
            )
        }
        None => DEFAULT_SIZE,
    };
    WindowBounds::centered(
        size(px(width.max(MIN_SIZE.0)), px(height.max(MIN_SIZE.1))),
        cx,
    )
}

/// 现在有没有屏幕可以放窗口。
///
/// 所有显示器都在休眠、或者正在插拔重新配置时，系统会报告「没有屏幕」，而 GPUI 在这时取主屏
/// 会直接 panic（`no primary screen`）。开窗口的调用多半来自系统的事件回调（点 Dock 图标、
/// 点菜单栏项），panic 沿着系统的 C 帧传出去就是整个进程 abort，这正是线上出现过的崩溃：
/// 所以开窗口之前先问一句，没有屏幕就放弃这一次，用户再点一下即可。
pub fn screens_available(cx: &App) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        objc2::MainThreadMarker::new()
            .is_some_and(|mtm| objc2_app_kit::NSScreen::screens(mtm).count() > 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        !cx.displays().is_empty()
    }
}

pub fn open_main_window(cx: &mut App) {
    if !screens_available(cx) {
        tracing::warn!("no display available, not opening the main window");
        return;
    }
    let state = AppState::global(cx);
    let options = WindowOptions {
        window_bounds: Some(initial_bounds(&state, cx)),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        app_id: Some(APP_ID.into()),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(&t!("app_name"));
        let state = state.clone();
        cx.new(|cx| Workspace::new(state, window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            // 窗口可能关了很久，先把会变的几项补采一遍。
            state.update(cx, |s, cx| {
                s.main_window = Some(handle);
                s.sync_feeds(cx);
                s.refresh_live(cx);
            });
            cx.activate(true);
        }
        Err(e) => tracing::error!("failed to open main window: {e}"),
    }
}

pub fn show_main_window(cx: &mut App) {
    // Dock 图标、托盘、第二个实例都会走到这里，其中 Dock 是系统的 Apple Event 回调直接进来的：
    // panic 在那里会让整个进程 abort。出了问题就放弃这一次「打开窗口」，下一次还能再试。
    guarded("show main window", || {
        let state = AppState::global(cx);
        match state.read(cx).main_window {
            Some(handle) if window_alive(cx, handle) => {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
                cx.activate(true);
            }
            _ => open_main_window(cx),
        }
    });
}

/// 真正退出：先保存窗口位置与设置。
pub fn quit(cx: &mut App) {
    // 存盘只是顺手的事，它出了问题也必须能退出。
    guarded("save before quit", || {
        let state = AppState::global(cx);
        if let Some(handle) = state.read(cx).main_window
            && window_alive(cx, handle)
        {
            let _ = handle.update(cx, |_, window, cx| remember_bounds(&state, window, cx));
        }
        if let Some(handle) = state.read(cx).floating_window
            && window_alive(cx, handle)
        {
            let _ = handle.update(cx, |_, window, cx| crate::floating::remember(window, cx));
        }
        if let Some(handle) = state.read(cx).wall_window
            && window_alive(cx, handle)
        {
            let _ = handle.update(cx, |_, window, cx| {
                crate::bigscreen::remember(&state, window, cx)
            });
        }
        state.read(cx).settings.save();
    });
    cx.quit();
}

/// 窗口被关闭之后：清理句柄；没有任何理由继续驻留就退出进程。
pub fn on_window_closed(cx: &mut App) {
    guarded("window closed", || window_closed(cx));
}

fn window_closed(cx: &mut App) {
    let state = AppState::global(cx);
    let detail_gone = state
        .read(cx)
        .detail_window
        .is_some_and(|h| !window_alive(cx, h));
    if detail_gone {
        state.update(cx, |s, cx| {
            s.detail_closed = s
                .detail_metric
                .map(|metric| (metric, std::time::Instant::now()));
            s.detail_window = None;
            s.detail_metric = None;
            s.highlight_metric(None);
            s.sync_feeds(cx);
            cx.notify();
        });
    }
    let (main_gone, wall_gone, floating_gone, keep) = {
        let s = state.read(cx);
        (
            s.main_window.is_some_and(|h| !window_alive(cx, h)),
            s.wall_window.is_some_and(|h| !window_alive(cx, h)),
            s.floating_window.is_some_and(|h| !window_alive(cx, h)),
            s.keeps_running_when_closed(),
        )
    };
    if floating_gone {
        // 用户双击悬浮窗把它关了：让设置开关同步回「关」。
        state.update(cx, |s, cx| {
            s.floating_window = None;
            s.update_settings(|settings| settings.floating_net = false, cx);
        });
    }
    if wall_gone {
        // 大屏关了（位置在关窗之前就记下了）：补存一次设置，进程采样按需要收缩。
        state.update(cx, |s, cx| {
            s.wall_window = None;
            s.sync_feeds(cx);
            s.settings.save();
            cx.notify();
        });
    }
    if main_gone {
        state.update(cx, |s, cx| {
            s.main_window = None;
            s.sync_feeds(cx);
            s.settings.save();
        });
    }
    // 主窗口和大屏都关了、又不驻留托盘，就没有理由继续运行。大屏还开着的时候，关掉主窗口不退出。
    if (main_gone || wall_gone) && !keep {
        let (main_open, wall_open) = {
            let s = state.read(cx);
            (
                s.main_window.is_some_and(|h| window_alive(cx, h)),
                s.wall_window.is_some_and(|h| window_alive(cx, h)),
            )
        };
        if !main_open && !wall_open {
            quit(cx);
        }
    }
}
