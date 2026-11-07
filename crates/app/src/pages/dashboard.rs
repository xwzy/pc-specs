//! 概览：实时指标卡片 + 硬件摘要。

use super::chart;
use crate::health::{self, Level};
use crate::state::{AppState, Page};
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    rems,
};
use pc_specs_core::model::NetworkInterface;
use rust_i18n::t;

/// 走势卡片里展示最近多少个采样。
const SPARK_POINTS: usize = 60;

pub fn render(s: &AppState, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let alerts = s.alerts();
    let level = health::overall(&alerts);
    let snap = &s.snapshot;

    // 副标题：机型 · 系统 · 运行时长
    let mut subtitle = Vec::new();
    if let Some(model) = snap.motherboard.as_ref().and_then(|m| m.model.clone()) {
        subtitle.push(model);
    }
    if !snap.os.name.is_empty() {
        subtitle.push(
            format!("{} {}", snap.os.name, snap.os.version)
                .trim()
                .to_string(),
        );
    }
    if snap.host.uptime_secs > 0 {
        subtitle.push(format!("↑ {}", fmt::duration(snap.host.uptime_secs)));
    }
    let subtitle = (!subtitle.is_empty()).then(|| SharedString::from(subtitle.join(" · ")));

    let health_label = match level {
        Level::Healthy => t!("dash_health_healthy"),
        Level::Warning => t!("dash_health_warning"),
        Level::Critical => t!("dash_health_critical"),
    }
    .into_owned();

    // ---- 实时卡片 ----
    let settings = &s.settings;
    let palette = chart::palette(cx);
    let mut tiles: Vec<AnyElement> = Vec::new();

    let (cpu_pct, mem_pct) = s
        .latest
        .as_ref()
        .map(|t| {
            (
                t.cpu_overall,
                fmt::ratio(t.mem_used_bytes, t.mem_total_bytes),
            )
        })
        .unwrap_or((
            snap.cpu.usage_overall,
            fmt::ratio(snap.memory.used_bytes, snap.memory.total_bytes),
        ));

    tiles.push(tile(
        "cpu",
        IconName::Cpu,
        t!("dash_card_cpu").into_owned(),
        fmt::percent(cpu_pct),
        cpu_detail(s),
        Some(cpu_pct),
        Some(chart::sparkline(
            "dash-spark-cpu",
            chart::series(s, Some(SPARK_POINTS), |t| t.cpu_overall as f64),
            Some((0.0, 100.0)),
            palette[0],
        )),
        Page::Cpu,
        state,
        cx,
    ));

    tiles.push(tile(
        "memory",
        IconName::MemoryStick,
        t!("dash_card_memory").into_owned(),
        fmt::percent(mem_pct),
        format!(
            "{} / {}",
            settings.bytes(
                s.latest
                    .as_ref()
                    .map_or(snap.memory.used_bytes, |t| t.mem_used_bytes)
            ),
            settings.bytes(snap.memory.total_bytes)
        ),
        Some(mem_pct),
        Some(chart::sparkline(
            "dash-spark-mem",
            chart::series(s, Some(SPARK_POINTS), |t| {
                fmt::ratio(t.mem_used_bytes, t.mem_total_bytes) as f64
            }),
            Some((0.0, 100.0)),
            palette[1],
        )),
        Page::Memory,
        state,
        cx,
    ));

    if let Some(disk) = primary_storage(s) {
        let pct = fmt::ratio(disk.used_bytes, disk.total_bytes);
        let io = s
            .latest
            .as_ref()
            .map(|t| (t.disk_read_bps, t.disk_write_bps))
            .unwrap_or_default();
        tiles.push(tile(
            "storage",
            IconName::HardDrive,
            t!("dash_card_storage").into_owned(),
            fmt::percent(pct),
            format!(
                "{} / {} · R {} W {}",
                settings.bytes(disk.used_bytes),
                settings.bytes(disk.total_bytes),
                settings.rate(io.0),
                settings.rate(io.1)
            ),
            Some(pct),
            None,
            Page::Storage,
            state,
            cx,
        ));
    }

    let (rx, tx) = s
        .latest
        .as_ref()
        .map(|t| (t.net_rx_bps, t.net_tx_bps))
        .unwrap_or_default();
    let net_unit = settings.net_unit;
    tiles.push(tile(
        "network",
        IconName::Network,
        t!("dash_card_network").into_owned(),
        format!("↓ {}", settings.rate(rx)),
        format!("↑ {}", settings.rate(tx)),
        None,
        Some(chart::sparkline(
            "dash-spark-net",
            chart::series(s, Some(SPARK_POINTS), move |t| {
                fmt::rate_chart_value(t.net_rx_bps, net_unit)
            }),
            None,
            palette[2],
        )),
        Page::Network,
        state,
        cx,
    ));

    let hottest = s.latest.as_ref().and_then(|t| t.max_temperature());
    tiles.push(tile(
        "temperature",
        IconName::Thermometer,
        t!("dash_card_sensors").into_owned(),
        hottest.map_or("—".to_string(), |v| settings.temp(v)),
        if hottest.is_some() {
            t!("dash_hottest").into_owned()
        } else {
            t!("dash_no_temp").into_owned()
        },
        hottest.map(|v| v.clamp(0.0, 100.0)),
        None,
        Page::Sensors,
        state,
        cx,
    ));

    let tile_row = h_flex()
        .w_full()
        .flex_wrap()
        .items_stretch()
        .gap_4()
        .children(tiles);

    // ---- 告警条 ----
    let alert_row = if alerts.is_empty() {
        h_flex()
            .gap_2()
            .items_center()
            .text_color(cx.theme().success)
            .child(Icon::new(IconName::CircleCheck).small())
            .child(div().text_sm().child(t!("dash_all_normal").into_owned()))
            .into_any_element()
    } else {
        h_flex()
            .gap_2()
            .flex_wrap()
            .children(
                alerts
                    .iter()
                    .map(|a| ui::level_tag(a.level, s.alert_message(a)).into_any_element()),
            )
            .into_any_element()
    };

    // ---- 硬件摘要 ----
    let summary = ui::section("dash-summary", t!("dash_summary_title").into_owned())
        .child(v_flex().gap_1().children(summary_rows(s, state, cx)));

    ui::page(
        "page-dashboard",
        t!("nav_dashboard").into_owned(),
        subtitle,
        vec![ui::level_tag(level, health_label).into_any_element()],
        vec![
            alert_row,
            tile_row.into_any_element(),
            summary.into_any_element(),
        ],
        cx,
    )
}

/// 一张实时指标卡片，点击跳转到对应页面。
#[allow(clippy::too_many_arguments)]
fn tile(
    key: &'static str,
    icon: IconName,
    label: String,
    value: String,
    detail: String,
    percent: Option<f32>,
    spark: Option<AnyElement>,
    target: Page,
    state: &Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let state = state.clone();
    let ring = cx.theme().ring;
    v_flex()
        .id(SharedString::from(format!("tile-{key}")))
        .flex_1()
        .min_w(rems(15.))
        .gap_2()
        .p_4()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().tokens.group_box)
        .cursor_pointer()
        .hover(move |this| this.border_color(ring))
        .on_click(move |_, _, cx| {
            state.update(cx, |s, cx| s.set_page(target, cx));
        })
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(ui::icon_label(icon, label, cx))
                .child(div().text_xl().font_semibold().child(value)),
        )
        .when_some(percent, |this, p| {
            this.child(ui::usage_bar(SharedString::from(format!("tile-bar-{key}")), p, cx).small())
        })
        .when_some(spark, |this, spark| {
            this.child(div().h_12().w_full().child(spark))
        })
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child(detail),
        )
        .into_any_element()
}

fn cpu_detail(s: &AppState) -> String {
    let cpu = &s.snapshot.cpu;
    match &cpu.topology {
        Some(t) if t.p_cores.is_some() && t.e_cores.is_some() => t!(
            "dash_cores_short",
            p = t.p_cores.unwrap_or_default(),
            e = t.e_cores.unwrap_or_default()
        )
        .into_owned(),
        _ if cpu.logical_cores > 0 => t!("dash_cores_simple", n = cpu.logical_cores).into_owned(),
        _ => String::new(),
    }
}

/// 容量最大的存储卷作为「主存储」。
pub fn primary_storage(s: &AppState) -> Option<&pc_specs_core::model::StorageInfo> {
    s.snapshot.storages.iter().max_by_key(|d| d.total_bytes)
}

/// 优先选已连接、有 IPv4 的物理接口。
pub fn primary_interface(s: &AppState) -> Option<&NetworkInterface> {
    s.snapshot
        .network
        .interfaces
        .iter()
        .filter(|i| i.is_up && !i.is_loopback && !i.ipv4.is_empty())
        .find(|i| matches!(i.kind.as_str(), "ethernet" | "wifi"))
        .or_else(|| {
            s.snapshot
                .network
                .interfaces
                .iter()
                .find(|i| i.is_up && !i.is_loopback && !i.ipv4.is_empty())
        })
}

fn summary_rows(s: &AppState, state: &Entity<AppState>, cx: &App) -> Vec<AnyElement> {
    let snap = &s.snapshot;
    let settings = &s.settings;
    let mut rows: Vec<(Page, IconName, String, String)> = Vec::new();

    if !snap.cpu.brand.is_empty() {
        let cores = if snap.cpu.physical_cores > 0 {
            format!(
                " · {}C/{}T",
                snap.cpu.physical_cores, snap.cpu.logical_cores
            )
        } else {
            String::new()
        };
        rows.push((
            Page::Cpu,
            IconName::Cpu,
            t!("dash_summary_cpu").into_owned(),
            format!("{}{}", snap.cpu.brand, cores),
        ));
    }
    if snap.memory.total_bytes > 0 {
        let module = snap.memory.modules.first();
        let extra = module
            .map(|m| {
                [m.kind.clone(), m.speed_mt_s.map(|v| format!("{v} MT/s"))]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|v| !v.is_empty())
            .map(|v| format!(" · {v}"))
            .unwrap_or_default();
        rows.push((
            Page::Memory,
            IconName::MemoryStick,
            t!("dash_summary_memory").into_owned(),
            format!("{}{}", settings.bytes(snap.memory.total_bytes), extra),
        ));
    }
    if !snap.gpus.is_empty() {
        rows.push((
            Page::Gpu,
            IconName::Gpu,
            t!("dash_summary_gpu").into_owned(),
            snap.gpus
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if let Some(disk) = primary_storage(s) {
        rows.push((
            Page::Storage,
            IconName::HardDrive,
            t!("dash_summary_storage").into_owned(),
            format!(
                "{} · {} · {}",
                disk.name,
                settings.bytes(disk.total_bytes),
                disk.kind
            ),
        ));
    }
    if let Some(mb) = &snap.motherboard {
        let text = [mb.vendor.clone(), mb.model.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            rows.push((
                Page::Motherboard,
                IconName::CircuitBoard,
                t!("dash_summary_motherboard").into_owned(),
                text,
            ));
        }
    }
    if !snap.os.name.is_empty() {
        rows.push((
            Page::Os,
            IconName::Laptop,
            t!("dash_summary_os").into_owned(),
            format!("{} {} · {}", snap.os.name, snap.os.version, snap.os.arch),
        ));
    }
    if let Some(d) = snap
        .displays
        .iter()
        .find(|d| d.is_primary)
        .or(snap.displays.first())
    {
        let hz = d
            .refresh_hz
            .map(|v| format!(" @ {v} Hz"))
            .unwrap_or_default();
        rows.push((
            Page::Display,
            IconName::Monitor,
            t!("dash_summary_display").into_owned(),
            format!("{}×{}{}", d.width_px, d.height_px, hz),
        ));
    }
    if let Some(i) = primary_interface(s) {
        rows.push((
            Page::Network,
            IconName::Network,
            t!("dash_summary_network").into_owned(),
            format!(
                "{} · {}",
                i.name,
                i.ipv4.first().cloned().unwrap_or_default()
            ),
        ));
    }
    if let Some(b) = &snap.battery {
        rows.push((
            Page::Battery,
            IconName::BatteryCharging,
            t!("dash_summary_battery").into_owned(),
            format!("{:.0}% · {}", b.percentage, b.state),
        ));
    }

    rows.into_iter()
        .map(|(page, icon, label, value)| {
            let state = state.clone();
            let hover_bg = cx.theme().list_hover;
            h_flex()
                .id(SharedString::from(format!("summary-{}", page.key())))
                .items_center()
                .gap_3()
                .px_2()
                .py_2()
                .rounded(cx.theme().radius)
                .cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| s.set_page(page, cx));
                })
                .child(
                    h_flex()
                        .w(rems(10.))
                        .flex_shrink_0()
                        .child(ui::icon_label(icon, label, cx)),
                )
                .child(div().flex_1().min_w_0().truncate().child(value))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element()
        })
        .collect()
}
