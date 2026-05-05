//! 系统托盘：实时状态展示 + 控制菜单。
//!
//! 菜单结构（动态依赖 [`TraySettings`]）：
//!
//! ```text
//! ─ CPU    24% · 4.0 GHz                <metric, disabled>
//! ─ MEM    38 / 64 GB (59%)
//! ─ DISK   ↓12 MB/s ↑4 MB/s
//! ─ NET    ↓2.4 MB/s ↑0.3 MB/s
//! ─ TEMP   max 56°C
//! ──────────────────
//! Show / Hide
//! ──────────────────
//! Quit
//! ```
//!
//! - 每个 monitor tick 调 [`on_tick`]，**不重建菜单**，仅 `set_text` 更新各 metric
//!   行的文字；这是低开销方案（Tauri 内部直接打 native API）。
//! - 用户改设置时，前端走 invoke `apply_tray_settings`，后端调 [`apply_settings`]
//!   重建菜单（开销可忽略，只在用户操作时触发）。
//! - macOS 额外支持 `tray.set_title("↓2.4M ↑0.3M")` 让系统状态栏图标旁直接显示
//!   实时网速文本（iStat Menus 同款体验）；其他平台是 no-op。
//!
//! 全局状态用 `OnceCell<Mutex<...>>`：进程内只有一个托盘，不需要每个 AppHandle
//! 各自一份。

use crate::model::MonitorTick;
use parking_lot::Mutex;
use std::sync::OnceLock;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, Wry,
};

const TRAY_ID: &str = "pc-specs-tray";
const TRAY_TOOLTIP_FALLBACK: &str = "PC Specs";

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TraySettings {
    pub show_cpu: bool,
    pub show_memory: bool,
    pub show_disk: bool,
    pub show_network: bool,
    pub show_temperature: bool,
    /// macOS 状态栏图标旁是否显示实时网速文字。其他平台忽略。
    pub macos_show_title: bool,
}

impl Default for TraySettings {
    fn default() -> Self {
        Self {
            show_cpu: true,
            show_memory: true,
            show_disk: false,
            show_network: true,
            show_temperature: true,
            macos_show_title: false,
        }
    }
}

/// 各 metric 在菜单里的句柄。重建菜单时整体替换，平时由 [`on_tick`]
/// 调用 `set_text` 原地更新文字。
struct MetricItems {
    cpu: Option<MenuItem<Wry>>,
    mem: Option<MenuItem<Wry>>,
    disk: Option<MenuItem<Wry>>,
    net: Option<MenuItem<Wry>>,
    temp: Option<MenuItem<Wry>>,
}

struct TrayInner {
    settings: TraySettings,
    metrics: MetricItems,
    /// 上一次接收到的 tick，重建菜单时用来填充初始文本，避免显示空白行。
    last_tick: Option<MonitorTick>,
}

static STATE: OnceLock<Mutex<TrayInner>> = OnceLock::new();

fn state() -> &'static Mutex<TrayInner> {
    STATE.get_or_init(|| {
        Mutex::new(TrayInner {
            settings: TraySettings::default(),
            metrics: MetricItems {
                cpu: None,
                mem: None,
                disk: None,
                net: None,
                temp: None,
            },
            last_tick: None,
        })
    })
}

pub fn install(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => {
            tracing::warn!("no default window icon; tray will be installed without icon");
            return Ok(());
        }
    };

    let menu = {
        let mut g = state().lock();
        let TrayInner {
            settings,
            metrics,
            last_tick,
        } = &mut *g;
        build_menu(app, settings, metrics, last_tick.as_ref())?
    };

    TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(TRAY_TOOLTIP_FALLBACK)
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray.show" => show_main_window(app),
            "tray.hide" => hide_main_window(app),
            "tray.quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// monitor 模块每次 tick 调用一次。开销 = 5 次 set_text + 1 次 set_tooltip
/// （+ macOS 下 1 次 set_title），数十微秒级。
pub fn on_tick<R: Runtime>(app: &AppHandle<R>, tick: &MonitorTick) {
    let mut g = state().lock();
    g.last_tick = Some(tick.clone());
    let s = g.settings;

    if let Some(item) = &g.metrics.cpu {
        let _ = item.set_text(format_cpu_line(tick));
    }
    if let Some(item) = &g.metrics.mem {
        let _ = item.set_text(format_mem_line(tick));
    }
    if let Some(item) = &g.metrics.disk {
        let _ = item.set_text(format_disk_line(tick));
    }
    if let Some(item) = &g.metrics.net {
        let _ = item.set_text(format_net_line(tick));
    }
    if let Some(item) = &g.metrics.temp {
        let _ = item.set_text(format_temp_line(tick));
    }
    drop(g);

    let tooltip = format_tooltip(tick);
    let title = format_macos_title(tick);

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip));
        // set_title 在 Linux/Windows 上是 no-op（参考 tauri 文档），调它无副作用。
        // macOS 上若用户在 settings 关掉 macos_show_title，则传 None 清空文字。
        let title_arg: Option<&str> = if s.macos_show_title { Some(&title) } else { None };
        let _ = tray.set_title(title_arg);
    }
}

/// 用户改设置时调用：重建菜单，并立刻用 last_tick 填充一遍文字。
pub fn apply_settings(app: &AppHandle<Wry>, new_settings: TraySettings) -> tauri::Result<()> {
    let tray = match app.tray_by_id(TRAY_ID) {
        Some(t) => t,
        None => {
            tracing::warn!("tray not yet installed; skipping apply_settings");
            return Ok(());
        }
    };
    let (menu, replay_tick) = {
        let mut g = state().lock();
        g.settings = new_settings;
        let TrayInner {
            settings,
            metrics,
            last_tick,
        } = &mut *g;
        let menu = build_menu(app, settings, metrics, last_tick.as_ref())?;
        (menu, last_tick.clone())
    };
    tray.set_menu(Some(menu))?;
    if let Some(tick) = replay_tick {
        on_tick(app, &tick);
    }
    if !new_settings.macos_show_title {
        let _ = tray.set_title(None::<&str>);
    }
    Ok(())
}

fn build_menu(
    app: &AppHandle<Wry>,
    settings: &TraySettings,
    metrics: &mut MetricItems,
    last_tick: Option<&MonitorTick>,
) -> tauri::Result<Menu<Wry>> {
    // 全部新建一组 MenuItem（disabled，作为信息行）。Tauri 不允许同一个
    // MenuItem 实例被绑到两个 Menu，所以重建菜单时必须重建所有 item。
    metrics.cpu = if settings.show_cpu {
        Some(make_disabled_item(
            app,
            "tray.metric.cpu",
            &last_tick.map(format_cpu_line).unwrap_or_else(empty_cpu),
        )?)
    } else {
        None
    };
    metrics.mem = if settings.show_memory {
        Some(make_disabled_item(
            app,
            "tray.metric.mem",
            &last_tick.map(format_mem_line).unwrap_or_else(empty_mem),
        )?)
    } else {
        None
    };
    metrics.disk = if settings.show_disk {
        Some(make_disabled_item(
            app,
            "tray.metric.disk",
            &last_tick.map(format_disk_line).unwrap_or_else(empty_disk),
        )?)
    } else {
        None
    };
    metrics.net = if settings.show_network {
        Some(make_disabled_item(
            app,
            "tray.metric.net",
            &last_tick.map(format_net_line).unwrap_or_else(empty_net),
        )?)
    } else {
        None
    };
    metrics.temp = if settings.show_temperature {
        Some(make_disabled_item(
            app,
            "tray.metric.temp",
            &last_tick.map(format_temp_line).unwrap_or_else(empty_temp),
        )?)
    } else {
        None
    };

    let show = MenuItem::with_id(app, "tray.show", "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "tray.hide", "Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray.quit", "Quit", true, None::<&str>)?;

    let mut items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = Vec::new();
    if let Some(i) = &metrics.cpu {
        items.push(i);
    }
    if let Some(i) = &metrics.mem {
        items.push(i);
    }
    if let Some(i) = &metrics.disk {
        items.push(i);
    }
    if let Some(i) = &metrics.net {
        items.push(i);
    }
    if let Some(i) = &metrics.temp {
        items.push(i);
    }
    let any_metric = !items.is_empty();
    let sep_top = PredefinedMenuItem::separator(app)?;
    let sep_bottom = PredefinedMenuItem::separator(app)?;
    if any_metric {
        items.push(&sep_top);
    }
    items.push(&show);
    items.push(&hide);
    items.push(&sep_bottom);
    items.push(&quit);

    Menu::with_items(app, &items)
}

fn make_disabled_item(
    app: &AppHandle<Wry>,
    id: &str,
    text: &str,
) -> tauri::Result<MenuItem<Wry>> {
    // 第 4 个参数是 enabled。传 false → 灰显且点击无反应，作为纯信息行；
    // text 后续可由 on_tick 调 `MenuItem::set_text` 原地更新。
    MenuItem::with_id(app, id, text, false, None::<&str>)
}

// ---------- 文案 ---------------------------------------------------------
// 全部用英文，跨平台 / 跨语言托盘里不期望出现 CJK 长串（macOS 状态栏宽度有限）。

fn format_cpu_line(t: &MonitorTick) -> String {
    format!("CPU   {:>4.0}%", t.cpu_overall)
}

fn empty_cpu() -> String {
    "CPU   —".to_string()
}

fn format_mem_line(t: &MonitorTick) -> String {
    if t.mem_total_bytes == 0 {
        return "MEM   —".to_string();
    }
    let pct = (t.mem_used_bytes as f64) / (t.mem_total_bytes as f64) * 100.0;
    format!(
        "MEM   {} / {}  ({:>3.0}%)",
        human_bytes(t.mem_used_bytes),
        human_bytes(t.mem_total_bytes),
        pct
    )
}

fn empty_mem() -> String {
    "MEM   —".to_string()
}

fn format_disk_line(t: &MonitorTick) -> String {
    format!(
        "DISK  ↓{}/s  ↑{}/s",
        human_bytes(t.disk_read_bps),
        human_bytes(t.disk_write_bps),
    )
}

fn empty_disk() -> String {
    "DISK  —".to_string()
}

fn format_net_line(t: &MonitorTick) -> String {
    format!(
        "NET   ↓{}/s  ↑{}/s",
        human_bytes(t.net_rx_bps),
        human_bytes(t.net_tx_bps),
    )
}

fn empty_net() -> String {
    "NET   —".to_string()
}

fn format_temp_line(t: &MonitorTick) -> String {
    let max = t
        .temperatures
        .iter()
        .filter(|s| s.kind == "temperature")
        .map(|s| s.value)
        .fold(f32::NAN, |a, b| if a.is_nan() || b > a { b } else { a });
    if max.is_nan() {
        return "TEMP  —".to_string();
    }
    format!("TEMP  max {:>3.0}°C", max)
}

fn empty_temp() -> String {
    "TEMP  —".to_string()
}

fn format_tooltip(t: &MonitorTick) -> String {
    let mem_pct = if t.mem_total_bytes > 0 {
        (t.mem_used_bytes as f64) / (t.mem_total_bytes as f64) * 100.0
    } else {
        0.0
    };
    format!(
        "PC Specs · CPU {:.0}% · MEM {:.0}% · ↓{}/s ↑{}/s",
        t.cpu_overall,
        mem_pct,
        human_bytes(t.net_rx_bps),
        human_bytes(t.net_tx_bps),
    )
}

/// macOS 状态栏文字模式：极简紧凑 `↓2.4M ↑0.3M`，给状态栏宽度预算。
fn format_macos_title(t: &MonitorTick) -> String {
    format!(
        "↓{} ↑{}",
        human_bytes_compact(t.net_rx_bps),
        human_bytes_compact(t.net_tx_bps),
    )
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n == 0 {
        return "0 B".to_string();
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[i])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

/// 紧凑单位（用于 macOS title 字数限制）：1.2M / 8K / 240B
fn human_bytes_compact(n: u64) -> String {
    if n < 1024 {
        return format!("{n}B");
    }
    let kb = n as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.0}K");
    }
    let mb = kb / 1024.0;
    if mb < 1024.0 {
        if mb >= 100.0 {
            format!("{mb:.0}M")
        } else {
            format!("{mb:.1}M")
        }
    } else {
        let gb = mb / 1024.0;
        format!("{gb:.1}G")
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn hide_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        if visible {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}
