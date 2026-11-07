//! 应用级共享模型。
//!
//! 一个 `Entity<AppState>` 持有设置、硬件快照、实时采样历史、进程列表等全部共享数据；
//! 所有页面都是它的只读视图，交互通过它的方法修改。窗口只是视图，关闭窗口不会丢状态，
//! 这也是「关闭到托盘」得以成立的基础。
//!
//! 线程约定：系统信息采集全是阻塞调用，一律放到独立的 `std::thread`，结果经
//! `async_channel` 回到界面线程。`Sampler` 在 Windows 上不是 `Send`，所以必须在监控线程内创建。

use crate::bigscreen::data::History;
use crate::health::{self, Alert, AlertKind, Level};
use crate::menubar::archive::Archive;
use crate::menubar::data::{self as menu_data, Sample};
use crate::poster;
use crate::settings::{Settings, ThemePref};
use crate::tray::{Tray, TrayEvent};
use gpui_kit::assets::IconName;
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{Theme, ThemeMode, WindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClipboardItem, Context, Entity, Global, SharedString,
    SystemNotification,
};
use pc_specs_core::exporter;
use pc_specs_core::model::{MonitorTick, ProcessInfo, SystemSnapshot};
use pc_specs_core::modules::{self, Module, ModuleData};
use pc_specs_core::monitor::{ProcessSampler, Sampler, TEMPERATURE_TTL};
use pc_specs_core::server::{self, ServerHandle, ServerStatus};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// 全局句柄：让设置页的 getter / setter 闭包（只拿得到 `&App`）也能访问共享模型。
pub struct Hub(pub Entity<AppState>);
impl Global for Hub {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Dashboard,
    Cpu,
    Gpu,
    Memory,
    Storage,
    Motherboard,
    Display,
    Network,
    Battery,
    Sensors,
    Peripherals,
    Os,
    DevEnv,
    Monitor,
    MenuBar,
    BigScreen,
    Processes,
    Export,
    Settings,
}

impl Page {
    pub const HARDWARE: [Page; 10] = [
        Page::Cpu,
        Page::Gpu,
        Page::Memory,
        Page::Storage,
        Page::Motherboard,
        Page::Display,
        Page::Network,
        Page::Battery,
        Page::Sensors,
        Page::Peripherals,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Page::Dashboard => "dashboard",
            Page::Cpu => "cpu",
            Page::Gpu => "gpu",
            Page::Memory => "memory",
            Page::Storage => "storage",
            Page::Motherboard => "motherboard",
            Page::Display => "display",
            Page::Network => "network",
            Page::Battery => "battery",
            Page::Sensors => "sensors",
            Page::Peripherals => "peripherals",
            Page::Os => "os",
            Page::DevEnv => "dev-env",
            Page::Monitor => "monitor",
            Page::MenuBar => "menu-bar",
            Page::BigScreen => "big-screen",
            Page::Processes => "processes",
            Page::Export => "export",
            Page::Settings => "settings",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Page::Dashboard => IconName::LayoutDashboard,
            Page::Cpu => IconName::Cpu,
            Page::Gpu => IconName::Gpu,
            Page::Memory => IconName::MemoryStick,
            Page::Storage => IconName::HardDrive,
            Page::Motherboard => IconName::CircuitBoard,
            Page::Display => IconName::Monitor,
            Page::Network => IconName::Network,
            Page::Battery => IconName::BatteryCharging,
            Page::Sensors => IconName::Thermometer,
            Page::Peripherals => IconName::Usb,
            Page::Os => IconName::Laptop,
            Page::DevEnv => IconName::Terminal,
            Page::Monitor => IconName::Activity,
            Page::MenuBar => IconName::ChartNoAxesCombined,
            Page::BigScreen => IconName::MonitorPlay,
            Page::Processes => IconName::List,
            Page::Export => IconName::FileDown,
            Page::Settings => IconName::Settings,
        }
    }

    pub fn title(self) -> String {
        let title = match self {
            Page::Dashboard => t!("nav_dashboard"),
            Page::Cpu => t!("nav_cpu"),
            Page::Gpu => t!("nav_gpu"),
            Page::Memory => t!("nav_memory"),
            Page::Storage => t!("nav_storage"),
            Page::Motherboard => t!("nav_motherboard"),
            Page::Display => t!("nav_display"),
            Page::Network => t!("nav_network"),
            Page::Battery => t!("nav_battery"),
            Page::Sensors => t!("nav_sensors"),
            Page::Peripherals => t!("nav_peripherals"),
            Page::Os => t!("nav_os"),
            Page::DevEnv => t!("nav_dev_env"),
            Page::Monitor => t!("nav_monitor"),
            Page::MenuBar => t!("bar_title"),
            Page::BigScreen => t!("bs_title"),
            Page::Processes => t!("nav_processes"),
            Page::Export => t!("nav_export"),
            Page::Settings => t!("nav_settings"),
        };
        title.into_owned()
    }

    /// 该页面对应的快照模块；用于显示「加载中」骨架。
    pub fn module(self) -> Option<Module> {
        Some(match self {
            Page::Cpu => Module::Cpu,
            Page::Gpu => Module::Gpu,
            Page::Memory => Module::Memory,
            Page::Storage => Module::Storage,
            Page::Motherboard => Module::Motherboard,
            Page::Display => Module::Display,
            Page::Network => Module::Network,
            Page::Battery => Module::Battery,
            Page::Sensors => Module::Sensors,
            Page::Peripherals => Module::Peripherals,
            Page::Os => Module::Os,
            Page::DevEnv => Module::DevEnv,
            _ => return None,
        })
    }
}

/// 公网 IP 查询状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupState {
    Idle,
    Loading,
    /// `None` 表示查询失败。
    Done(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Json,
}

pub struct ExportState {
    pub format: ExportFormat,
    pub text: Option<SharedString>,
    pub poster_busy: bool,
}

enum MonitorCmd {
    Interval(u64),
    /// 温度传感器的读取周期，`None` 表示不读（见 `AppState::temperature_demand`）。
    Temperature(Option<Duration>),
}

/// 只有告警在用温度（没人看得见读数）时，多久读一次传感器。
/// 告警要的是「持续过热」，十几秒的分辨率足够。
const ALERT_TEMPERATURE_EVERY: Duration = Duration::from_secs(10);

/// 连续多少个采样超标才算「持续」告警（CPU 瞬时尖峰很常见）。
const SUSTAINED_TICKS: u32 = 5;
/// 同一类告警两次系统通知之间的最短间隔。
const ALERT_COOLDOWN: Duration = Duration::from_secs(300);

/// 会随时间变化、又不在实时采样里的模块（运行时长、磁盘用量、网卡、电池）。
/// 常驻托盘时快照会越来越旧，所以定期只重采这几个，而不是整个快照。
const LIVE_MODULES: [Module; 4] = [
    Module::Host,
    Module::Storage,
    Module::Network,
    Module::Battery,
];
/// 一轮硬件采集最多算多久：超过了就当有模块卡死，放开「正在刷新」的状态。
const REFRESH_TIMEOUT: Duration = Duration::from_secs(180);
/// 窗口开着时的重采间隔。
const LIVE_REFRESH_VISIBLE: Duration = Duration::from_secs(30);
/// 窗口关着时只是为了让磁盘告警别太旧，慢一点。
const LIVE_REFRESH_HIDDEN: Duration = Duration::from_secs(300);

pub struct AppState {
    pub settings: Settings,
    pub page: Page,

    pub snapshot: SystemSnapshot,
    pub loaded: HashSet<Module>,
    pub refreshing: bool,
    refresh_epoch: u64,
    live_refreshing: bool,
    /// 每次轻量刷新加一，让它的超时看门狗认得出「是不是还是自己那一轮」。
    live_serial: u64,
    /// 开机自启是否已登记（以系统为准，见 `autostart`）。
    pub autostart: bool,

    /// 实时采样历史（环形缓冲），最新的在尾部。
    pub ticks: VecDeque<MonitorTick>,
    /// 菜单栏面板的走势历史，和 `ticks` 各记各的（见 `push_tick`）。
    pub menu_history: VecDeque<Sample>,
    /// 监控大屏的走势历史：同样各记各的，暂停 / 清空监控页不影响大屏。
    pub wall: History,
    /// 菜单栏面板的长期走势（1 小时到 30 天），按设置存在磁盘上，见 `menubar::archive`。
    pub archive: Archive,
    /// 与 `ticks` 一一对应的时间标签（`HH:MM:SS`），图表横轴用，推入时生成一次。
    pub tick_labels: VecDeque<SharedString>,
    pub latest: Option<MonitorTick>,
    pub paused: bool,
    monitor_tx: mpsc::Sender<MonitorCmd>,
    /// 上次告诉监控线程的温度读取周期。
    temperature_demand: Option<Duration>,

    pub processes: Vec<ProcessInfo>,
    /// 进程列表每更新一次加一，让进程页只在数据真的变了时才重排。
    pub process_rev: u64,
    process_stop: Option<mpsc::Sender<()>>,
    process_epoch: u64,

    pub public_ip: LookupState,
    pub export: ExportState,

    pub server: Option<ServerHandle>,
    server_epoch: u64,

    pub main_window: Option<AnyWindowHandle>,
    pub floating_window: Option<AnyWindowHandle>,
    /// 监控大屏窗口。它是独立的看板，可以在主窗口关掉之后单独留着。
    pub wall_window: Option<AnyWindowHandle>,
    pub detail_window: Option<AnyWindowHandle>,
    pub detail_metric: Option<crate::menubar::config::Metric>,
    /// 弹窗刚被关掉的指标和时间：点击菜单栏项会先让弹窗失焦关闭、再触发点击，
    /// 靠它分辨「这一下是想把弹窗收起来」，免得刚关就又打开。
    pub detail_closed: Option<(crate::menubar::config::Metric, Instant)>,
    tray: Option<Tray>,
    tray_events: async_channel::Sender<TrayEvent>,

    alert_streak: HashMap<AlertKind, u32>,
    alert_notified: HashMap<AlertKind, Instant>,
}

impl AppState {
    pub fn global(cx: &App) -> Entity<AppState> {
        cx.global::<Hub>().0.clone()
    }

    pub fn new(settings: Settings, cx: &mut Context<Self>) -> Self {
        let (monitor_tx, monitor_rx) = mpsc::channel();
        let (tick_tx, tick_rx) = async_channel::bounded::<MonitorTick>(1);
        let interval = Duration::from_millis(settings.interval_ms);
        spawn_thread("pc-specs-monitor", move || {
            run_monitor(monitor_rx, tick_tx, interval)
        });
        cx.spawn(async move |this, cx| {
            while let Ok(tick) = tick_rx.recv().await {
                // 这个任务由系统的事件循环直接驱动，里面 panic 就是整个进程 abort：
                // 一次推送出了问题，丢掉这一帧，下一帧照常。
                if this
                    .update(cx, |this, cx| {
                        guarded("push tick", || this.push_tick(tick, cx));
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (tray_tx, tray_rx) = async_channel::unbounded::<TrayEvent>();
        let tray = if settings.tray_enabled {
            Tray::new(&settings, tray_tx.clone(), None, &SystemSnapshot::default())
        } else {
            None
        };
        cx.spawn(async move |this, cx| {
            while let Ok(event) = tray_rx.recv().await {
                if this.upgrade().is_none() {
                    break;
                }
                // 窗口操作要读写 `AppState`，所以不能放进它自己的 update 里。
                cx.update(|cx| {
                    guarded("tray event", || match event {
                        TrayEvent::Show => crate::workspace::show_main_window(cx),
                        TrayEvent::Quit => crate::workspace::quit(cx),
                        TrayEvent::Configure => {
                            let state = AppState::global(cx);
                            state.update(cx, |s, cx| s.set_page(Page::MenuBar, cx));
                            crate::workspace::show_main_window(cx);
                        }
                        TrayEvent::BigScreen => crate::bigscreen::open(cx),
                        TrayEvent::Panel => crate::menubar::panel::toggle(AppState::global(cx), cx),
                        TrayEvent::Inspect(metric, anchor) => {
                            let state = AppState::global(cx);
                            let anchor = anchor.or_else(|| {
                                state
                                    .read(cx)
                                    .tray
                                    .as_ref()
                                    .and_then(|tray| tray.anchor(metric))
                            });
                            crate::menubar::panel::show(state, metric, anchor, cx);
                        }
                    });
                });
            }
        })
        .detach();

        let archive = Archive::open(settings.history_persist);
        let mut this = Self {
            settings,
            page: Page::Dashboard,
            snapshot: SystemSnapshot::default(),
            loaded: HashSet::new(),
            refreshing: false,
            refresh_epoch: 0,
            live_refreshing: false,
            live_serial: 0,
            autostart: crate::autostart::is_enabled(),
            ticks: VecDeque::new(),
            menu_history: VecDeque::new(),
            wall: History::default(),
            archive,
            tick_labels: VecDeque::new(),
            latest: None,
            paused: false,
            monitor_tx,
            // 监控线程启动时按这个节奏读；第一个采样之后会按实际需要调整。
            temperature_demand: Some(TEMPERATURE_TTL),
            processes: Vec::new(),
            process_rev: 0,
            process_stop: None,
            process_epoch: 0,
            public_ip: LookupState::Idle,
            export: ExportState {
                format: ExportFormat::Markdown,
                text: None,
                poster_busy: false,
            },
            server: None,
            server_epoch: 0,
            main_window: None,
            floating_window: None,
            wall_window: None,
            detail_window: None,
            detail_metric: None,
            detail_closed: None,
            tray,
            tray_events: tray_tx,
            alert_streak: HashMap::new(),
            alert_notified: HashMap::new(),
        };
        this.apply_language(cx);
        this.apply_theme(cx);
        this.restart_server(cx);
        this.refresh(cx);

        // 退出前把还没落盘的长期历史写下去（包括正在积累的最后一分钟）。
        cx.on_app_quit(|this, _| {
            guarded("save history", || this.archive.shutdown());
            async {}
        })
        .detach();

        cx.spawn(async move |this, cx| {
            loop {
                let visible = this.read_with(cx, |this, _| {
                    this.main_window.is_some() || this.wall_window.is_some()
                });
                let Ok(visible) = visible else { break };
                let wait = if visible {
                    LIVE_REFRESH_VISIBLE
                } else {
                    LIVE_REFRESH_HIDDEN
                };
                cx.background_executor().timer(wait).await;
                if this.update(cx, |this, cx| this.refresh_live(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    // ---- 设置 -----------------------------------------------------------

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        let before = self.settings.clone();
        f(&mut self.settings);
        self.settings.menu_bar.normalize();
        self.settings.bigscreen.normalize();
        if self.settings == before {
            return;
        }
        let now = self.settings.clone();
        if now.language != before.language {
            self.apply_language(cx);
        }
        if now.theme != before.theme {
            self.apply_theme(cx);
        }
        if now.interval_ms != before.interval_ms {
            let _ = self.monitor_tx.send(MonitorCmd::Interval(now.interval_ms));
        }
        if now.interval_ms != before.interval_ms || now.history_minutes != before.history_minutes {
            self.trim_history();
        }
        if now.tray_enabled != before.tray_enabled
            || now.language != before.language
            || now.menu_bar != before.menu_bar
            || now.temp_unit != before.temp_unit
            || now.net_unit != before.net_unit
        {
            self.sync_tray();
        }
        if now.history_persist != before.history_persist {
            self.archive.set_persist(now.history_persist);
        }
        if now.floating_net != before.floating_net {
            self.sync_floating(cx);
        }
        // 大屏上的进程组件开关了，进程采样跟着启停。
        let wants_processes = |s: &Settings| {
            s.bigscreen
                .uses(crate::bigscreen::config::WidgetKind::Processes)
        };
        if wants_processes(&now) != wants_processes(&before) {
            self.sync_process_feed(cx);
        }
        if now.server_enabled != before.server_enabled || now.server_lan != before.server_lan {
            self.restart_server(cx);
        }
        self.settings.save();
        cx.notify();
    }

    fn apply_language(&mut self, cx: &mut Context<Self>) {
        let code = self.settings.language.code();
        rust_i18n::set_locale(code);
        gpui_kit::component::set_locale(code);
        crate::workspace::install_menus(cx);
        cx.refresh_windows();
    }

    pub fn apply_theme(&mut self, cx: &mut Context<Self>) {
        match self.settings.theme {
            ThemePref::Dark => Theme::change(ThemeMode::Dark, None, cx),
            ThemePref::Light => Theme::change(ThemeMode::Light, None, cx),
            ThemePref::System => Theme::sync_system_appearance(None, cx),
        }
    }

    // ---- 页面 -----------------------------------------------------------

    pub fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.sync_process_feed(cx);
        // 每次进入导出页都按最新数据生成一份预览。
        if page == Page::Export {
            self.generate_export(self.export.format, cx);
        }
        cx.notify();
    }

    // ---- 硬件快照 ---------------------------------------------------------

    /// 重新采集所有模块。哪个模块先完成就先落到界面上。
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refresh_epoch += 1;
        let epoch = self.refresh_epoch;
        self.refreshing = true;
        // 已加载的页面继续展示旧值，避免每次刷新都切回骨架屏。

        let (tx, rx) = async_channel::unbounded::<(Module, ModuleData)>();
        let started = spawn_thread("pc-specs-collect", move || {
            // CPU 使用率要在我们自己还没 spawn 一堆子进程时测，所以先单独采它。
            let _ = tx.send_blocking((Module::Cpu, modules::collect(Module::Cpu)));
            std::thread::scope(|scope| {
                for module in Module::ALL.into_iter().filter(|m| *m != Module::Cpu) {
                    let tx = tx.clone();
                    scope.spawn(move || {
                        let _ = tx.send_blocking((module, modules::collect(module)));
                    });
                }
            });
        });
        if !started {
            self.refreshing = false;
            return;
        }

        cx.spawn(async move |this, cx| {
            let mut remaining = Module::ALL.len();
            while let Ok((module, data)) = rx.recv().await {
                remaining = remaining.saturating_sub(1);
                let alive = this.update(cx, |this, cx| {
                    if this.refresh_epoch != epoch {
                        return;
                    }
                    guarded("apply module", || {
                        this.snapshot.apply(data);
                        if let Some(tick) = &this.latest {
                            this.snapshot.merge_tick(tick);
                        }
                    });
                    this.loaded.insert(module);
                    if remaining == 0 {
                        this.refreshing = false;
                        this.snapshot.timestamp = pc_specs_core::util::now_ms();
                    }
                    cx.notify();
                });
                if alive.is_err() || remaining == 0 {
                    break;
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.refresh_epoch == epoch {
                    this.refreshing = false;
                    cx.notify();
                }
            });
        })
        .detach();
        // 某个模块卡死（比如问一个连不上的网络卷）时，「正在刷新」不能永远亮着，
        // 否则手动刷新和定时的轻量刷新都会被它挡住。晚到的结果带着旧的 epoch，会被丢掉。
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                if this.refresh_epoch == epoch && this.refreshing {
                    tracing::warn!("hardware refresh timed out, releasing the refreshing flag");
                    this.refreshing = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 只重采 `LIVE_MODULES`。不碰 `loaded` / `refreshing`，页面不会闪「加载中」。
    pub fn refresh_live(&mut self, cx: &mut Context<Self>) {
        if self.refreshing || self.live_refreshing {
            return;
        }
        self.live_refreshing = true;
        self.live_serial += 1;
        let serial = self.live_serial;
        let epoch = self.refresh_epoch;
        let (tx, rx) = async_channel::unbounded::<ModuleData>();
        let started = spawn_thread("pc-specs-live", move || {
            std::thread::scope(|scope| {
                for module in LIVE_MODULES {
                    let tx = tx.clone();
                    scope.spawn(move || {
                        let _ = tx.send_blocking(modules::collect(module));
                    });
                }
            });
        });
        if !started {
            self.live_refreshing = false;
            return;
        }

        cx.spawn(async move |this, cx| {
            let mut remaining = LIVE_MODULES.len();
            while let Ok(data) = rx.recv().await {
                remaining = remaining.saturating_sub(1);
                let alive = this.update(cx, |this, cx| {
                    // 手动全量刷新开始后，不允许较早的定时采集覆盖新结果。
                    if this.refresh_epoch != epoch {
                        return;
                    }
                    guarded("apply module", || {
                        this.snapshot.apply(data);
                        if let Some(tick) = &this.latest {
                            this.snapshot.merge_tick(tick);
                        }
                    });
                    if remaining == 0 {
                        this.live_refreshing = false;
                    }
                    cx.notify();
                });
                if alive.is_err() || remaining == 0 {
                    break;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.live_refreshing = false;
                cx.notify();
            });
        })
        .detach();
        // 同 `refresh`：别让一个卡死的模块把定时刷新永远挡住。
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_TIMEOUT).await;
            let _ = this.update(cx, |this, _| {
                if this.live_serial == serial {
                    this.live_refreshing = false;
                }
            });
        })
        .detach();
    }

    pub fn is_loaded(&self, module: Module) -> bool {
        self.loaded.contains(&module)
    }

    pub fn clear_public_ip(&mut self, cx: &mut Context<Self>) {
        self.snapshot.network.public_ip = None;
        self.public_ip = LookupState::Idle;
        cx.notify();
    }

    pub fn fetch_public_ip(&mut self, cx: &mut Context<Self>) {
        if self.public_ip == LookupState::Loading {
            return;
        }
        self.public_ip = LookupState::Loading;
        cx.notify();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(modules::network::fetch_public_ip());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(ip) = rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.snapshot.network.public_ip = ip.clone();
                    this.public_ip = LookupState::Done(ip);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    // ---- 实时采样 ---------------------------------------------------------

    /// 现在有没有人需要温度、需要多频繁。读温度是整个采样里最重的一项
    /// （macOS 上每次要和 hidd 往返通信，Windows 上要走 WMI），没人看的时候就别读。
    fn temperature_demand(&self) -> Option<Duration> {
        let on_menu_bar = self.tray.is_some()
            && self
                .settings
                .menu_bar
                .visible()
                .any(|item| item.metric == crate::menubar::config::Metric::Temperature);
        // 窗口、菜单栏弹窗、监控大屏开着，或者菜单栏上有温度项：读数有人看，按正常节奏读。
        if self.main_window.is_some()
            || self.detail_window.is_some()
            || self.wall_window.is_some()
            || on_menu_bar
        {
            Some(TEMPERATURE_TTL)
        } else if self.settings.alerts {
            Some(ALERT_TEMPERATURE_EVERY)
        } else {
            None
        }
    }

    fn sync_temperature_demand(&mut self) {
        let demand = self.temperature_demand();
        if demand != self.temperature_demand {
            self.temperature_demand = demand;
            let _ = self.monitor_tx.send(MonitorCmd::Temperature(demand));
        }
    }

    fn push_tick(&mut self, tick: MonitorTick, cx: &mut Context<Self>) {
        self.sync_temperature_demand();
        // 把磁盘 / 网卡 / GPU 的实时读数写回快照，各页面直接读快照即可。
        self.snapshot.merge_tick(&tick);
        // 菜单栏面板的走势单独记：「暂停」只冻结实时监控页的图表。
        let sample = Sample::new(&tick, &self.snapshot, &self.settings);
        // 睡眠、合盖之后醒来：中间那段没有采样，实时走势不该把两头连成一条线，从醒来这一刻重画。
        let gap_ms = (self.settings.interval_ms * 4).max(5_000);
        if self
            .menu_history
            .back()
            .is_some_and(|last| sample.timestamp.saturating_sub(last.timestamp) > gap_ms)
        {
            self.menu_history.clear();
        }
        self.archive
            .record(sample.timestamp, |channel| sample.value(channel));
        self.menu_history.push_back(sample);
        while self.menu_history.len() > menu_data::HISTORY {
            self.menu_history.pop_front();
        }
        self.wall.push(&tick);
        if let Some(tray) = self.tray.as_mut() {
            tray.on_tick(&tick, &self.settings, &self.snapshot);
        }
        if self.settings.alerts {
            self.evaluate_alerts(&tick, cx);
        }
        if !self.paused {
            self.tick_labels
                .push_back(crate::fmt::clock(tick.timestamp).into());
            self.ticks.push_back(tick.clone());
            self.trim_history();
        }
        self.latest = Some(tick);
        cx.notify();
    }

    fn trim_history(&mut self) {
        crate::history::trim(
            &mut self.ticks,
            &mut self.tick_labels,
            Duration::from_secs(u64::from(self.settings.history_minutes) * 60),
        );
    }

    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.ticks.clear();
        self.tick_labels.clear();
        cx.notify();
    }

    /// 清空菜单栏面板的全部走势：长期历史（内存和磁盘）和最近的实时走势。
    pub fn clear_archive(&mut self, cx: &mut Context<Self>) {
        self.archive.clear();
        self.menu_history.clear();
        cx.notify();
    }

    /// 面板里所有走势图一起切换的回看时长。
    pub fn set_panel_range(
        &mut self,
        range: crate::menubar::config::TimeRange,
        cx: &mut Context<Self>,
    ) {
        self.update_settings(|s| s.panel_range = range, cx);
    }

    pub fn toggle_paused(&mut self, cx: &mut Context<Self>) {
        self.paused = !self.paused;
        cx.notify();
    }

    /// 当前健康告警。
    pub fn alerts(&self) -> Vec<Alert> {
        health::evaluate(self.latest.as_ref(), &self.snapshot.storages)
    }

    fn evaluate_alerts(&mut self, tick: &MonitorTick, cx: &mut Context<Self>) {
        let alerts = health::evaluate(Some(tick), &self.snapshot.storages);
        let critical: HashMap<AlertKind, &Alert> = alerts
            .iter()
            .filter(|a| a.level == Level::Critical)
            .map(|a| (a.kind, a))
            .collect();
        for kind in [
            AlertKind::Cpu,
            AlertKind::Memory,
            AlertKind::Disk,
            AlertKind::Temperature,
        ] {
            let Some(alert) = critical.get(&kind) else {
                self.alert_streak.remove(&kind);
                continue;
            };
            let streak = self.alert_streak.entry(kind).or_insert(0);
            *streak += 1;
            if *streak < SUSTAINED_TICKS {
                continue;
            }
            let cooled_down = self
                .alert_notified
                .get(&kind)
                .is_none_or(|at| at.elapsed() >= ALERT_COOLDOWN);
            if cooled_down {
                self.alert_notified.insert(kind, Instant::now());
                let message = self.alert_message(alert);
                self.notify_alert(message, cx);
            }
        }
    }

    pub fn alert_message(&self, alert: &Alert) -> String {
        let message = match alert.kind {
            AlertKind::Cpu => t!("alert_cpu", value = format!("{:.0}", alert.value)),
            AlertKind::Memory => t!("alert_mem", value = format!("{:.0}", alert.value)),
            AlertKind::Disk => t!(
                "alert_disk",
                name = alert.detail.clone().unwrap_or_default(),
                value = format!("{:.0}", alert.value)
            ),
            AlertKind::Temperature => {
                t!("alert_temp", value = self.settings.temp(alert.value))
            }
        };
        message.into_owned()
    }

    // ---- 进程列表 ---------------------------------------------------------

    pub fn sync_process_feed(&mut self, cx: &mut Context<Self>) {
        if (self.page == Page::Processes && self.main_window.is_some())
            || (self.detail_window.is_some()
                && self.detail_metric.is_some_and(|m| m.needs_processes()))
            || (self.wall_window.is_some()
                && self
                    .settings
                    .bigscreen
                    .uses(crate::bigscreen::config::WidgetKind::Processes))
        {
            if self.process_stop.is_none() {
                self.start_process_feed(cx);
            }
        } else {
            self.stop_process_feed();
        }
    }

    fn start_process_feed(&mut self, cx: &mut Context<Self>) {
        self.stop_process_feed();
        let epoch = self.process_epoch;
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (tx, rx) = async_channel::bounded::<Vec<ProcessInfo>>(1);
        let started = spawn_thread("pc-specs-processes", move || {
            let mut sampler = guarded("process sampler init", ProcessSampler::new);
            // 基线建好后尽快出第一帧（CPU 占用至少要隔一小段才有意义），之后按 1.5 秒一帧。
            let mut wait = Duration::from_millis(400);
            // 超时 = 该采样了；收到停止信号或发送端断开则退出。
            while let Err(RecvTimeoutError::Timeout) = stop_rx.recv_timeout(wait) {
                if sampler.is_none() {
                    sampler = guarded("process sampler init", ProcessSampler::new);
                }
                let list = sampler
                    .as_mut()
                    .and_then(|sampler| guarded("process sampler", || sampler.sample()));
                match list {
                    Some(list) => {
                        if tx.send_blocking(list).is_err() {
                            break;
                        }
                        wait = Duration::from_millis(1500);
                    }
                    // 采样器 panic 过：丢掉它，隔几秒换个新的重来，这一帧就让它空着。
                    None => {
                        sampler = None;
                        wait = SAMPLER_RETRY;
                    }
                }
            }
        });
        if started {
            self.process_stop = Some(stop_tx);
        } else {
            return;
        }
        cx.spawn(async move |this, cx| {
            while let Ok(list) = rx.recv().await {
                let accepted = this
                    .update(cx, |this, cx| {
                        if this.process_epoch != epoch {
                            return false;
                        }
                        this.processes = list;
                        this.process_rev += 1;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !accepted {
                    break;
                }
            }
        })
        .detach();
    }

    fn stop_process_feed(&mut self) {
        // 丢弃 Sender 即通知线程退出。
        self.process_stop = None;
        self.process_epoch = self.process_epoch.wrapping_add(1);
    }

    // ---- 本地 HTTP 服务 -----------------------------------------------------

    pub fn restart_server(&mut self, cx: &mut Context<Self>) {
        self.server_epoch += 1;
        let epoch = self.server_epoch;
        let old = self.server.take();
        let enabled = self.settings.server_enabled;
        let ip = if self.settings.server_lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        let addr = SocketAddr::new(ip, server::DEFAULT_PORT);
        let (tx, rx) = async_channel::bounded(1);
        // 关闭旧服务会等端口释放，新服务要等它释放完才能绑定，所以两步放在同一个后台线程里。
        std::thread::spawn(move || {
            drop(old);
            let handle = enabled.then(|| server::spawn(addr));
            let _ = tx.send_blocking(handle);
        });
        cx.spawn(async move |this, cx| {
            let Ok(handle) = rx.recv().await else { return };
            let accepted = this
                .update(cx, |this, cx| {
                    if this.server_epoch != epoch {
                        return Some(handle);
                    }
                    this.server = handle;
                    cx.notify();
                    None
                })
                .ok()
                .flatten();
            if let Some(stale) = accepted {
                std::thread::spawn(move || drop(stale));
                return;
            }
            // 启动是异步的：轮询几次，把 Starting 刷新成 Listening / Failed。
            for _ in 0..10 {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        this.server_epoch != epoch
                            || this
                                .server
                                .as_ref()
                                .is_none_or(|s| s.status() != ServerStatus::Starting)
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn server_status(&self) -> Option<ServerStatus> {
        self.server.as_ref().map(|s| s.status())
    }

    // ---- 导出 -------------------------------------------------------------

    /// 带有最新实时读数的快照副本。
    pub fn export_snapshot(&self) -> SystemSnapshot {
        let mut snapshot = self.snapshot.clone();
        if let Some(tick) = &self.latest {
            snapshot.merge_tick(tick);
        }
        snapshot.timestamp = pc_specs_core::util::now_ms();
        snapshot
    }

    pub fn generate_export(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let snapshot = self.export_snapshot();
        let sensitive = self.settings.export_sensitive;
        let text = match format {
            ExportFormat::Markdown => exporter::to_markdown(&snapshot, sensitive),
            ExportFormat::Json => exporter::to_json(&snapshot, true, sensitive),
        };
        self.export.format = format;
        self.export.text = Some(text.into());
        cx.notify();
    }

    /// 把预览文本存成文件（位置由系统保存对话框决定）。
    pub fn save_export(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.export.text.clone() else {
            return;
        };
        let ext = match self.export.format {
            ExportFormat::Markdown => "md",
            ExportFormat::Json => "json",
        };
        let name = format!(
            "pc-specs-{}.{ext}",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        self.save_with_dialog(name, false, move || Ok(text.as_bytes().to_vec()), cx);
    }

    /// 渲染 PNG 长图并存成文件。
    pub fn export_png(&mut self, cx: &mut Context<Self>) {
        if self.export.poster_busy {
            return;
        }
        let svg = poster::build_svg(&self.export_snapshot(), &self.settings);
        let name = format!(
            "pc-specs-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        self.save_with_dialog(name, true, move || poster::render_png(&svg), cx);
    }

    /// 先让用户选保存位置，再在后台线程里生成内容并写入，最后弹 toast 告知结果。
    fn save_with_dialog(
        &mut self,
        suggested: String,
        marks_busy: bool,
        produce: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let dir = dirs::download_dir()
            .or_else(dirs::document_dir)
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        let picked = cx.prompt_for_new_path(&dir, Some(&suggested));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            if marks_busy {
                let _ = this.update(cx, |this, cx| {
                    this.export.poster_busy = true;
                    cx.notify();
                });
            }
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let result = produce()
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
                    .map(|_| path);
                let _ = tx.send_blocking(result);
            });
            let Ok(result) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| {
                this.export.poster_busy = false;
                match result {
                    Ok(path) => this.toast(
                        NotificationType::Success,
                        t!("export_save_done", path = path.display().to_string()).into_owned(),
                        cx,
                    ),
                    Err(error) => this.toast(
                        NotificationType::Error,
                        t!("export_save_failed", error = error).into_owned(),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- 窗口与托盘 -------------------------------------------------------

    /// 悬浮窗的开关跟设置对齐。
    ///
    /// 开 / 关窗口都会同步触发渲染或「窗口已关闭」回调，而它们要读 `AppState`；
    /// 此刻我们正处在 `AppState` 自己的 update 里，直接做会重入 panic，所以推迟到本次 update 结束之后。
    pub fn sync_floating(&mut self, cx: &mut Context<Self>) {
        let state = cx.entity();
        if !self.settings.floating_net {
            if let Some(handle) = self.floating_window.take() {
                let state = cx.entity();
                cx.defer(move |cx| {
                    crate::floating::close(handle, cx);
                    // 设置在关窗之前就写过一次了，位置要补写一遍。
                    state.read(cx).settings.save();
                });
            }
            return;
        }
        cx.defer(move |cx| {
            let (wanted, open) = {
                let s = state.read(cx);
                (
                    s.settings.floating_net,
                    s.floating_window
                        .is_some_and(|handle| cx.windows().contains(&handle)),
                )
            };
            if wanted && !open {
                let saved = state.read(cx).settings.floating_pos;
                let handle = crate::floating::open(state.clone(), saved, cx);
                state.update(cx, |s, _| s.floating_window = handle);
            }
        });
    }

    /// 大屏窗口刚打开：把会变的几项补采一遍，温度和进程也按需要启动。
    pub fn wall_opened(&mut self, handle: AnyWindowHandle, cx: &mut Context<Self>) {
        self.wall_window = Some(handle);
        self.sync_temperature_demand();
        self.sync_process_feed(cx);
        self.refresh_live(cx);
        cx.notify();
    }

    fn sync_tray(&mut self) {
        if !self.settings.tray_enabled {
            self.tray = None;
        } else if let Some(tray) = self.tray.as_mut() {
            tray.apply_settings(&self.settings, self.latest.as_ref(), &self.snapshot);
        } else {
            self.tray = Tray::new(
                &self.settings,
                self.tray_events.clone(),
                self.latest.as_ref(),
                &self.snapshot,
            );
        }
    }

    /// 关闭窗口后进程是否继续驻留。
    pub fn keeps_running_when_closed(&self) -> bool {
        self.settings.close_to_tray && self.tray.is_some()
    }

    /// 弹窗打开期间让对应的菜单栏项保持高亮，关闭时传 `None`。
    pub fn highlight_metric(&mut self, metric: Option<crate::menubar::config::Metric>) {
        if let Some(tray) = self.tray.as_mut() {
            tray.highlight(metric);
        }
    }

    /// 托盘图标是否创建成功（没有的话，窗口关了就没法再叫出来）。
    pub fn has_tray(&self) -> bool {
        self.tray.is_some()
    }

    /// 开 / 关开机自启。结果以系统里实际的登记为准。
    pub fn set_autostart(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if let Err(error) = crate::autostart::set_enabled(enabled) {
            self.toast(
                NotificationType::Error,
                t!("autostart_failed", error = error).into_owned(),
                cx,
            );
        }
        self.autostart = crate::autostart::is_enabled();
        cx.notify();
    }

    // ---- 通知 -------------------------------------------------------------

    /// 复制到剪贴板并提示。
    pub fn copy_to_clipboard(&self, text: String, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.toast(
            NotificationType::Success,
            t!("common_copied").into_owned(),
            cx,
        );
    }

    /// 在主窗口里弹出 toast；窗口不存在时静默忽略。
    pub fn toast(&self, kind: NotificationType, message: impl Into<SharedString>, cx: &mut App) {
        let Some(handle) = self.main_window else {
            return;
        };
        let message = message.into();
        let _ = cx.update_window(handle, |_, window, cx| {
            window.push_notification((kind, message), cx);
        });
    }

    /// 告警：窗口开着就弹 toast，已关到托盘就走系统通知。
    fn notify_alert(&self, message: String, cx: &mut App) {
        let title: SharedString = t!("alert_title").into_owned().into();
        let open_window = self
            .main_window
            .filter(|handle| cx.windows().contains(handle));
        if let Some(handle) = open_window {
            let note = Notification::warning(message)
                .title(title)
                .in_app_and_system();
            let _ = cx.update_window(handle, |_, window, cx| window.push_notification(note, cx));
        } else {
            cx.show_system_notification(SystemNotification {
                tag: "pc-specs/alert".into(),
                title,
                body: message.into(),
                actions: Vec::new(),
            });
        }
    }
}

/// 起一个后台线程。系统线程数用尽时 `spawn` 会失败：那只是这个功能这一轮用不了，
/// 不该把整个进程带走。成功与否由返回值告诉调用方。
fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> bool {
    match std::thread::Builder::new().name(name.into()).spawn(f) {
        Ok(_) => true,
        Err(e) => {
            tracing::error!("failed to start the {name} thread: {e}");
            false
        }
    }
}

/// 采样器出了问题（panic）之后，隔多久再重建一个继续采。
const SAMPLER_RETRY: Duration = Duration::from_secs(2);

/// 监控线程：自己持有 `Sampler`，按间隔采样并把结果发回界面线程。
///
/// 采样器是对 sysinfo / 系统接口的一层封装，任何一次 panic 都不该让实时监控就此停摆：
/// 出事就丢掉旧的采样器，隔一会儿换个新的继续采。
fn run_monitor(
    commands: mpsc::Receiver<MonitorCmd>,
    ticks: async_channel::Sender<MonitorTick>,
    mut interval: Duration,
) {
    let mut temperature = Some(TEMPERATURE_TTL);
    let mut sampler = guarded("sampler init", Sampler::new);
    // 基线建立后至少等一个 CPU 最小刷新窗口，首个采样才有意义。
    let mut next = Instant::now() + Duration::from_millis(300);
    loop {
        match commands.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(MonitorCmd::Interval(ms)) => {
                let new_interval = Duration::from_millis(ms);
                // 间隔变短时尽快落到新节奏上；变长则保持当前计划，下次采样后生效。
                if new_interval < interval {
                    next = next.min(Instant::now() + new_interval);
                }
                interval = new_interval;
            }
            Ok(MonitorCmd::Temperature(every)) => {
                temperature = every;
                if let Some(sampler) = sampler.as_mut() {
                    sampler.set_temperature_interval(every);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if sampler.is_none() {
                    sampler = guarded("sampler init", Sampler::new);
                    if let Some(sampler) = sampler.as_mut() {
                        sampler.set_temperature_interval(temperature);
                    }
                }
                let tick = sampler
                    .as_mut()
                    .and_then(|sampler| guarded("sampler", || sampler.sample()));
                match tick {
                    Some(tick) => {
                        if ticks.send_blocking(tick).is_err() {
                            break;
                        }
                        next = Instant::now() + interval;
                    }
                    None => {
                        sampler = None;
                        next = Instant::now() + SAMPLER_RETRY;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
