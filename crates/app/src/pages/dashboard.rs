//! 概览：实时指标卡片 + 硬件摘要。

use super::chart;
use crate::state::{AppState, Page};
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    rems,
};
use pc_specs_core::model::{MonitorTick, NetworkInterface};
use rust_i18n::t;

/// 走势卡片里展示最近多少个采样。
const SPARK_POINTS: usize = 60;
/// 一张指标卡片的最小宽度（px），决定一行放几张。
const TILE_MIN_WIDTH: f32 = 250.0;

pub fn render(s: &AppState, state: &Entity<AppState>, content_width: f32, cx: &App) -> AnyElement {
    let alerts = s.alerts();
    let snap = &s.snapshot;

    // 副标题：机型 · 系统 · 已运行多久
    let mut subtitle = Vec::new();
    if let Some(model) = snap.motherboard.as_ref().and_then(|m| m.model.clone()) {
        subtitle.push(model);
    }
    if !snap.os.name.is_empty() {
        subtitle.push(fmt::os_title(&snap.os));
    }
    if snap.host.uptime_secs > 0 {
        subtitle.push(t!("dash_uptime", time = fmt::duration(snap.host.uptime_secs)).into_owned());
    }
    let subtitle = (!subtitle.is_empty()).then(|| SharedString::from(subtitle.join(" · ")));

    // ---- 实时卡片 ----
    let settings = &s.settings;
    let palette = chart::palette(cx);
    let latest = s.latest.as_ref();
    let mut tiles: Vec<AnyElement> = Vec::new();

    let (cpu_pct, mem_pct) = latest
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
        vec![fmt::core_summary(&snap.cpu)],
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

    let mem_used = latest.map_or(snap.memory.used_bytes, |t| t.mem_used_bytes);
    tiles.push(tile(
        "memory",
        IconName::MemoryStick,
        t!("dash_card_memory").into_owned(),
        fmt::percent(mem_pct),
        vec![format!(
            "{} / {}",
            settings.bytes(mem_used),
            settings.bytes(snap.memory.total_bytes)
        )],
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

    // GPU：只有读得到占用率时才放，免得出现一张永远是「—」的卡片。
    if let Some(gpu) = snap.gpus.iter().find(|g| g.utilization.is_some()) {
        let util_of = |tick: &MonitorTick, name: &str| {
            tick.gpus
                .iter()
                .find(|g| g.name == name)
                .and_then(|g| g.utilization)
        };
        let util = latest
            .and_then(|t| util_of(t, &gpu.name))
            .or(gpu.utilization)
            .unwrap_or(0.0);
        let name = gpu.name.clone();
        tiles.push(tile(
            "gpu",
            IconName::Gpu,
            t!("dash_card_gpu").into_owned(),
            fmt::percent(util),
            vec![gpu.name.clone()],
            Some(util),
            Some(chart::sparkline(
                "dash-spark-gpu",
                chart::series(s, Some(SPARK_POINTS), move |t| {
                    util_of(t, &name).unwrap_or(0.0) as f64
                }),
                Some((0.0, 100.0)),
                palette[3],
            )),
            Page::Gpu,
            state,
            cx,
        ));
    }

    if let Some(disk) = primary_storage(s) {
        let pct = fmt::ratio(disk.used_bytes, disk.total_bytes);
        let (read, write) = latest
            .map(|t| (t.disk_read_bps, t.disk_write_bps))
            .unwrap_or_default();
        tiles.push(tile(
            "storage",
            IconName::HardDrive,
            t!("dash_card_storage").into_owned(),
            fmt::percent(pct),
            vec![
                format!(
                    "{} / {}",
                    settings.bytes(disk.used_bytes),
                    settings.bytes(disk.total_bytes)
                ),
                format!(
                    "{} {} · {} {}",
                    t!("common_read"),
                    settings.rate(read),
                    t!("common_write"),
                    settings.rate(write)
                ),
            ],
            Some(pct),
            Some(chart::sparkline(
                "dash-spark-disk",
                chart::series(s, Some(SPARK_POINTS), |t| {
                    (t.disk_read_bps + t.disk_write_bps) as f64 / (1024.0 * 1024.0)
                }),
                None,
                palette[4],
            )),
            Page::Storage,
            state,
            cx,
        ));
    }

    let (rx, tx) = latest
        .map(|t| (t.net_rx_bps, t.net_tx_bps))
        .unwrap_or_default();
    let net_unit = settings.net_unit;
    tiles.push(tile(
        "network",
        IconName::Network,
        t!("dash_card_network").into_owned(),
        format!("↓ {}", settings.rate(rx)),
        vec![format!("↑ {}", settings.rate(tx))],
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

    // 温度：整机最热的那个传感器，卡片上写明是谁。
    let hottest = latest.and_then(|t| {
        t.temperatures
            .iter()
            .filter(|r| r.value.is_finite())
            .max_by(|a, b| a.value.total_cmp(&b.value))
    });
    let temp_unit = settings.temp_unit;
    tiles.push(tile(
        "temperature",
        IconName::Thermometer,
        t!("dash_card_temp").into_owned(),
        hottest.map_or("—".to_string(), |r| settings.temp(r.value)),
        vec![match hottest {
            Some(r) => format!("{} · {}", t!("dash_hottest"), fmt::sensor_label(r)),
            None => t!("dash_no_temp").into_owned(),
        }],
        hottest.map(|r| r.value.clamp(0.0, 100.0)),
        hottest.is_some().then(|| {
            chart::sparkline(
                "dash-spark-temp",
                chart::series(s, Some(SPARK_POINTS), move |t| {
                    t.max_temperature()
                        .map_or(0.0, |v| fmt::temp_chart_value(v, temp_unit))
                }),
                None,
                palette[4],
            )
        }),
        Page::Sensors,
        state,
        cx,
    ));

    // 等宽网格：整行放满，最后一行不会被拉宽，窗口窄了自动少放一列。
    let tile_grid = div()
        .w_full()
        .grid()
        .grid_cols(ui::columns(content_width, TILE_MIN_WIDTH, 3))
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

    let mut body = Vec::new();
    if !settings.tips_dismissed {
        body.push(tips_card(state, cx));
    }
    body.push(alert_row);
    body.push(tile_grid.into_any_element());
    body.push(summary.into_any_element());

    ui::page(
        "page-dashboard",
        t!("nav_dashboard").into_owned(),
        subtitle,
        vec![],
        body,
        cx,
    )
}

/// 新手提示：第一次打开时告诉用户，除了这个窗口，监控还能放到哪里。关掉后不再出现。
fn tips_card(state: &Entity<AppState>, cx: &App) -> AnyElement {
    let go = |id: &'static str, page: Page| {
        let state = state.clone();
        Button::new(id)
            .small()
            .outline()
            .icon(page.icon())
            .label(page.title())
            .on_click(move |_, _, cx| {
                state.update(cx, |s, cx| s.set_page(page, cx));
            })
    };
    let close_state = state.clone();
    h_flex()
        .items_start()
        .gap_3()
        .p_4()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().tokens.group_box)
        .child(
            Icon::new(IconName::Lightbulb)
                .small()
                .text_color(cx.theme().primary),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("dash_tip_title").into_owned()),
                )
                .child(ui::muted(t!("dash_tip_body").into_owned(), cx))
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .child(go("tip-menubar", Page::MenuBar))
                        .child(go("tip-bigscreen", Page::BigScreen))
                        .child(go("tip-settings", Page::Settings)),
                ),
        )
        .child(
            Button::new("tip-dismiss")
                .small()
                .ghost()
                .icon(IconName::X)
                .accessibility_label(t!("dash_tip_dismiss").into_owned())
                .tooltip(t!("dash_tip_dismiss").into_owned())
                .on_click(move |_, _, cx| {
                    close_state.update(cx, |s, cx| {
                        s.update_settings(|v| v.tips_dismissed = true, cx)
                    });
                }),
        )
        .into_any_element()
}

/// 一张实时指标卡片，点击跳转到对应页面。`details` 贴着卡片底部排列，
/// 同一行里高矮不一的卡片，说明文字仍然对齐。
#[allow(clippy::too_many_arguments)]
fn tile(
    key: &'static str,
    icon: IconName,
    label: String,
    value: String,
    details: Vec<String>,
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
        .min_w_0()
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
            v_flex()
                .mt_auto()
                .gap_0p5()
                .children(details.into_iter().map(|line| {
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(line)
                })),
        )
        .into_any_element()
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
            [
                Some(disk.name.clone()),
                Some(settings.bytes(disk.total_bytes)),
                fmt::storage_kind(&disk.kind),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
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
            format!("{} · {}", fmt::os_title(&snap.os), snap.os.arch),
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
            format!("{:.0}% · {}", b.percentage, fmt::battery_state(&b.state)),
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
