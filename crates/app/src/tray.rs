//! 系统托盘。GPUI 自己没有托盘 API，所以三个平台统一走 `tray-icon`
//! （Linux 底层是 StatusNotifierItem，不依赖 GTK）。
//!
//! 菜单：几行只读的实时指标 + 「显示窗口」+「退出」。菜单与图标事件在平台线程里触发，
//! 这里只往 channel 里丢一个 `TrayEvent`，由界面线程的 `AppState` 转成窗口操作。

use crate::menubar::{Anchor, config::Metric, data};
use crate::settings::Settings;
use pc_specs_core::model::{MonitorTick, SystemSnapshot};
use resvg::{tiny_skia, usvg};
use rust_i18n::t;
use std::time::{Duration, Instant};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const SHOW_ID: &str = "pc-specs.show";
const QUIT_ID: &str = "pc-specs.quit";
const CONFIG_ID: &str = "pc-specs.configure";

/// 采样周期有抖动，到点之前这么一点也算到了；不然 1 秒的刷新间隔会隔一次才刷新一回。
const REFRESH_SLACK: Duration = Duration::from_millis(150);

/// 主图标的悬停提示至少隔这么久才更新一次：提示只在鼠标停在图标上时才有人看，
/// 而每次更新在 Windows 是一次 `Shell_NotifyIcon`、在 Linux 是一次 D-Bus 通知。
const TOOLTIP_EVERY: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug)]
pub enum TrayEvent {
    Show,
    Quit,
    Configure,
    Inspect(Metric, Option<Anchor>),
}

pub struct Tray {
    icon: TrayIcon,
    /// 主图标现在是不是画在菜单栏上（macOS 上可以只留监控项）。
    icon_shown: bool,
    /// 只读的指标行，数量由设置决定；设置变化时整个菜单重建。
    metrics: Vec<MenuItem>,
    /// 指标行上现在显示的文字。没变的行不用再写一遍。
    lines: Vec<String>,
    tooltip: String,
    tooltip_at: Option<Instant>,
    last_refresh: Instant,
    #[cfg(target_os = "macos")]
    native: crate::menubar::native::NativeItems,
}

impl Tray {
    pub fn new(
        settings: &Settings,
        events: async_channel::Sender<TrayEvent>,
        latest: Option<&MonitorTick>,
        snapshot: &SystemSnapshot,
    ) -> Option<Tray> {
        if !host_available() {
            tracing::warn!("no StatusNotifier host on the session bus, skipping the system tray");
            return None;
        }
        install_handlers(events);
        // 菜单栏项的间距要在创建第一个状态项之前登记。
        #[cfg(target_os = "macos")]
        crate::menubar::native::prepare();
        let (menu, metrics) = build_menu(settings, latest, snapshot);
        #[cfg(target_os = "macos")]
        let native = {
            let mut native = crate::menubar::native::NativeItems::default();
            native.sync(settings, latest, snapshot);
            native
        };

        let builder = TrayIconBuilder::new()
            .with_id("pc-specs.main")
            .with_autosave_name("pc-specs.main")
            .with_menu(Box::new(menu))
            .with_tooltip(&t!("app_name"))
            // macOS 点击图标直接弹菜单；Windows 左键用来唤起窗口，右键才是菜单。
            .with_menu_on_left_click(cfg!(target_os = "macos"));
        let builder = match app_icon() {
            #[cfg(target_os = "macos")]
            Some(icon) => builder.with_icon_templated(icon),
            #[cfg(not(target_os = "macos"))]
            Some(icon) => builder.with_icon(icon),
            None => builder,
        };
        let icon = match builder.build() {
            Ok(icon) => icon,
            Err(e) => {
                tracing::warn!("system tray unavailable: {e}");
                return None;
            }
        };
        let mut tray = Tray {
            icon,
            icon_shown: true,
            metrics,
            lines: Vec::new(),
            tooltip: String::new(),
            tooltip_at: None,
            last_refresh: Instant::now(),
            #[cfg(target_os = "macos")]
            native,
        };
        tray.sync_icon(settings, true);
        tray.refresh(settings, latest, snapshot);
        Some(tray)
    }

    /// 主图标按设置显示或隐藏。macOS 上固定宽度，和监控项排在一起不显得松垮。
    fn sync_icon(&mut self, settings: &Settings, force: bool) {
        let visible = !cfg!(target_os = "macos") || settings.menu_bar.icon_visible();
        if visible == self.icon_shown && !force {
            return;
        }
        if visible != self.icon_shown {
            let _ = self.icon.set_visible(visible);
            self.icon_shown = visible;
        }
        #[cfg(target_os = "macos")]
        if visible && let Some(item) = self.icon.ns_status_item() {
            item.setLength(crate::menubar::native::MAIN_ICON_LENGTH);
        }
    }

    /// 设置（指标开关 / 语言）变了：菜单重建，再立刻补上一帧数据。
    pub fn apply_settings(
        &mut self,
        settings: &Settings,
        latest: Option<&MonitorTick>,
        snapshot: &SystemSnapshot,
    ) {
        let (menu, metrics) = build_menu(settings, latest, snapshot);
        self.icon.set_menu(Some(Box::new(menu)));
        self.metrics = metrics;
        // 新菜单的文字、提示都要重新写一遍。
        self.lines.clear();
        self.tooltip_at = None;
        let name = t!("app_name").into_owned();
        let _ = self.icon.set_tooltip(Some(name.as_str()));
        self.tooltip = name;
        self.sync_icon(settings, false);
        #[cfg(target_os = "macos")]
        self.native.sync(settings, latest, snapshot);
        self.refresh(settings, latest, snapshot);
    }

    pub fn on_tick(&mut self, tick: &MonitorTick, settings: &Settings, snapshot: &SystemSnapshot) {
        let interval = Duration::from_millis(settings.menu_bar.refresh_ms);
        if self.last_refresh.elapsed() < interval.saturating_sub(REFRESH_SLACK) {
            return;
        }
        // 屏幕都黑着的时候没人看菜单栏：不重绘。`last_refresh` 没动，醒来后的第一帧就会补上。
        #[cfg(target_os = "macos")]
        if crate::menubar::native::displays_asleep() {
            return;
        }
        self.refresh(settings, Some(tick), snapshot);
    }

    fn refresh(
        &mut self,
        settings: &Settings,
        tick: Option<&MonitorTick>,
        snapshot: &SystemSnapshot,
    ) {
        self.last_refresh = Instant::now();
        let lines = metric_lines(tick, settings, snapshot);
        for (index, (item, line)) in self.metrics.iter().zip(&lines).enumerate() {
            if self.lines.get(index) != Some(line) {
                item.set_text(line);
            }
        }
        let tooltip_due = self
            .tooltip_at
            .is_none_or(|at| at.elapsed() >= TOOLTIP_EVERY);
        if tick.is_some() && tooltip_due {
            let tooltip = std::iter::once(t!("app_name").into_owned())
                .chain(lines.iter().cloned())
                .collect::<Vec<_>>()
                .join("\n");
            // Windows 的托盘提示最多 127 个字符。
            let tooltip: String = tooltip.chars().take(120).collect();
            self.tooltip_at = Some(Instant::now());
            if tooltip != self.tooltip {
                let _ = self.icon.set_tooltip(Some(tooltip.as_str()));
                self.tooltip = tooltip;
            }
        }
        self.lines = lines;
        #[cfg(target_os = "macos")]
        self.native.refresh(settings, tick, snapshot);
    }

    pub fn anchor(&self, metric: Metric) -> Option<Anchor> {
        #[cfg(target_os = "macos")]
        {
            self.native.anchor(metric)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = metric;
            None
        }
    }

    /// 弹窗打开期间，对应的菜单栏项保持高亮。
    pub fn highlight(&mut self, metric: Option<Metric>) {
        #[cfg(target_os = "macos")]
        self.native.highlight(metric);
        #[cfg(not(target_os = "macos"))]
        let _ = metric;
    }
}

/// Linux 上无论有没有托盘宿主，`tray-icon` 都会「创建成功」。没有宿主（比如没装 AppIndicator
/// 扩展的 GNOME）时图标根本看不到，窗口一关应用就再也叫不出来了，所以先问一下 D-Bus。
/// 没有托盘就当作没有托盘：窗口关闭即退出，开机自启也会正常弹出窗口。
#[cfg(target_os = "linux")]
fn host_available() -> bool {
    use zbus::blocking::{Connection, Proxy};

    const WATCHER: &str = "org.kde.StatusNotifierWatcher";
    let Ok(connection) = Connection::session() else {
        return false;
    };
    Proxy::new(&connection, WATCHER, "/StatusNotifierWatcher", WATCHER)
        .and_then(|watcher| watcher.get_property::<bool>("IsStatusNotifierHostRegistered"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn host_available() -> bool {
    true
}

fn install_handlers(events: async_channel::Sender<TrayEvent>) {
    let menu_events = events.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let kind = match event.id.0.as_str() {
            SHOW_ID => TrayEvent::Show,
            QUIT_ID => TrayEvent::Quit,
            CONFIG_ID => TrayEvent::Configure,
            id => {
                let Some(metric) = Metric::ALL
                    .into_iter()
                    .find(|m| id == format!("pc-specs.inspect.{}", m.key()))
                else {
                    return;
                };
                TrayEvent::Inspect(metric, None)
            }
        };
        let _ = menu_events.try_send(kind);
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            id,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            if let Some(metric) = Metric::ALL
                .into_iter()
                .find(|m| id.0 == format!("pc-specs.metric.{}", m.key()))
            {
                #[cfg(target_os = "macos")]
                let anchor = crate::menubar::native::click_anchor();
                #[cfg(not(target_os = "macos"))]
                let anchor = None;
                let _ = events.try_send(TrayEvent::Inspect(metric, anchor));
            } else if !cfg!(target_os = "macos") {
                let _ = events.try_send(TrayEvent::Show);
            }
        }
    }));
}

fn build_menu(
    settings: &Settings,
    latest: Option<&MonitorTick>,
    snapshot: &SystemSnapshot,
) -> (Menu, Vec<MenuItem>) {
    let menu = Menu::new();
    let metrics: Vec<MenuItem> = metric_lines(latest, settings, snapshot)
        .iter()
        .zip(settings.menu_bar.visible())
        .map(|(line, item)| {
            MenuItem::with_id(
                format!("pc-specs.inspect.{}", item.metric.key()),
                line,
                true,
                None,
            )
        })
        .collect();
    for item in &metrics {
        let _ = menu.append(item);
    }
    if !metrics.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    let _ = menu.append(&MenuItem::with_id(SHOW_ID, &t!("tray_show"), true, None));
    let _ = menu.append(&MenuItem::with_id(
        CONFIG_ID,
        &t!("bar_configure"),
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(QUIT_ID, &t!("tray_quit"), true, None));
    (menu, metrics)
}

/// 按设置挑出要展示的指标。行数只取决于设置，不取决于数据，所以刷新时只改文字。
fn metric_lines(
    tick: Option<&MonitorTick>,
    settings: &Settings,
    snapshot: &SystemSnapshot,
) -> Vec<String> {
    settings
        .menu_bar
        .visible()
        .map(|item| {
            let reading = data::reading(item.metric, item.show, tick, snapshot, settings);
            format!("{}  {}", item.metric.title(), reading.plain())
        })
        .collect()
}

#[cfg(target_os = "macos")]
pub fn quick_menu() -> Menu {
    let menu = Menu::new();
    let _ = menu.append(&MenuItem::with_id(SHOW_ID, &t!("tray_show"), true, None));
    let _ = menu.append(&MenuItem::with_id(
        CONFIG_ID,
        &t!("bar_configure"),
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(QUIT_ID, &t!("tray_quit"), true, None));
    menu
}

// ---- 图标 -------------------------------------------------------------------
//
// 用一小段 SVG 现场栅格化，省得带二进制资源：macOS 要黑色单色「模板图」（系统按菜单栏
// 明暗自动染色），其它平台用带底色的彩色图标。

const TEMPLATE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 22 22">
<g fill="none" stroke="#000" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
<rect x="5" y="5" width="12" height="12" rx="2.2"/>
<path d="M8.5 2.2v2.8M13.5 2.2v2.8M8.5 17v2.8M13.5 17v2.8M2.2 8.5h2.8M2.2 13.5h2.8M17 8.5h2.8M17 13.5h2.8"/>
</g>
<rect x="8.6" y="8.6" width="4.8" height="4.8" rx="0.9" fill="#000"/>
</svg>"##;

const COLOR_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
<rect width="32" height="32" rx="7" fill="#2563eb"/>
<g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
<rect x="9" y="9" width="14" height="14" rx="2.5"/>
<path d="M13 5.5v3.5M19 5.5v3.5M13 23v3.5M19 23v3.5M5.5 13h3.5M5.5 19h3.5M23 13h3.5M23 19h3.5"/>
</g>
<rect x="13.2" y="13.2" width="5.6" height="5.6" rx="1" fill="#fff"/>
</svg>"##;

fn app_icon() -> Option<Icon> {
    let (svg, px) = if cfg!(target_os = "macos") {
        (TEMPLATE_SVG, 44)
    } else if cfg!(target_os = "windows") {
        (COLOR_SVG, 32)
    } else {
        (COLOR_SVG, 64)
    };
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia 是预乘 alpha，托盘要的是直通 alpha。
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Icon::from_rgba(rgba, px, px).ok()
}
