//! 实时监控：多张走势图 + 每核心占用。历史时长、暂停、清空都在这里操作。

use super::chart;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
    rems,
};
use rust_i18n::t;

/// 图表区的统一高度。
const CHART_HEIGHT: f32 = 10.0;

pub fn render(s: &AppState, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let settings = &s.settings;
    let palette = chart::palette(cx);
    let unit = settings.net_unit;
    // 纵轴的单位跟着这段时间里的峰值走：只有几十 KB/s 时别写成 0.03 MB/s。
    let net_scale = fmt::RateScale::new(
        s.ticks
            .iter()
            .map(|t| t.net_rx_bps.max(t.net_tx_bps))
            .max()
            .unwrap_or(0),
        unit,
    );
    let disk_scale = fmt::RateScale::new(
        s.ticks
            .iter()
            .map(|t| t.disk_read_bps.max(t.disk_write_bps))
            .max()
            .unwrap_or(0),
        unit,
    );
    let temp_unit = settings.temp_unit;
    let temp_unit_name = fmt::temp_unit_name(temp_unit);
    let latest = s.latest.as_ref();

    // ---- 操作区 ----
    let range_state = state.clone();
    let range = ui::segmented(
        "monitor-range",
        [1u32, 5, 10]
            .into_iter()
            .map(|m| (m, t!("unit_minutes", n = m).into_owned()))
            .collect(),
        settings.history_minutes,
        move |minutes, cx| {
            range_state.update(cx, |s, cx| {
                s.update_settings(|v| v.history_minutes = minutes, cx)
            });
        },
    )
    .into_any_element();

    let pause_state = state.clone();
    let pause = Button::new("monitor-pause")
        .small()
        .outline()
        .icon(if s.paused {
            IconName::Play
        } else {
            IconName::Pause
        })
        .label(if s.paused {
            t!("monitor_resume").into_owned()
        } else {
            t!("monitor_pause").into_owned()
        })
        .on_click(move |_, _, cx| {
            pause_state.update(cx, |s, cx| s.toggle_paused(cx));
        })
        .into_any_element();

    let clear_state = state.clone();
    let clear = Button::new("monitor-clear")
        .small()
        .outline()
        .icon(IconName::Trash)
        .label(t!("monitor_clear").into_owned())
        .on_click(move |_, _, cx| {
            clear_state.update(cx, |s, cx| s.clear_history(cx));
        })
        .into_any_element();

    // ---- 图表 ----
    let mut cards: Vec<AnyElement> = Vec::new();

    let cpu = latest.map_or(0.0, |t| t.cpu_overall);
    cards.push(chart_card(
        "monitor-cpu",
        t!("monitor_cpu_pct").into_owned(),
        fmt::percent(cpu),
        chart::area(
            "monitor-cpu-chart",
            chart::series(s, None, |t| t.cpu_overall as f64),
            Some((0.0, 100.0)),
            palette[0],
            |v| format!("{v:.0}%"),
        ),
        cx,
    ));

    let mem = latest.map_or(0.0, |t| fmt::ratio(t.mem_used_bytes, t.mem_total_bytes));
    cards.push(chart_card(
        "monitor-mem",
        t!("monitor_mem_pct").into_owned(),
        fmt::percent(mem),
        chart::area(
            "monitor-mem-chart",
            chart::series(s, None, |t| {
                fmt::ratio(t.mem_used_bytes, t.mem_total_bytes) as f64
            }),
            Some((0.0, 100.0)),
            palette[1],
            |v| format!("{v:.0}%"),
        ),
        cx,
    ));

    let (rx, tx) = latest.map_or((0, 0), |t| (t.net_rx_bps, t.net_tx_bps));
    cards.push(chart_card(
        "monitor-net",
        t!("monitor_net", unit = net_scale.name).into_owned(),
        format!("↓ {}  ↑ {}", settings.rate(rx), settings.rate(tx)),
        chart::area2(
            "monitor-net-chart",
            chart::series2(s, None, move |t| {
                (net_scale.value(t.net_rx_bps), net_scale.value(t.net_tx_bps))
            }),
            (palette[2], palette[3]),
            (
                t!("monitor_down").into_owned(),
                t!("monitor_up").into_owned(),
            ),
            fmt::axis_label,
            cx,
        ),
        cx,
    ));

    let (read, write) = latest.map_or((0, 0), |t| (t.disk_read_bps, t.disk_write_bps));
    cards.push(chart_card(
        "monitor-disk",
        t!("monitor_disk_io", unit = disk_scale.name).into_owned(),
        format!(
            "{} {}  {} {}",
            t!("common_read"),
            settings.rate(read),
            t!("common_write"),
            settings.rate(write)
        ),
        chart::area2(
            "monitor-disk-chart",
            chart::series2(s, None, move |t| {
                (
                    disk_scale.value(t.disk_read_bps),
                    disk_scale.value(t.disk_write_bps),
                )
            }),
            (palette[2], palette[3]),
            (
                t!("common_read").into_owned(),
                t!("common_write").into_owned(),
            ),
            fmt::axis_label,
            cx,
        ),
        cx,
    ));

    // 温度：整机最高值，以及（如果能识别）CPU 温度。
    let hottest = latest.and_then(|t| t.max_temperature());
    let has_temp = s.ticks.iter().any(|t| t.max_temperature().is_some());
    cards.push(if has_temp {
        chart_card(
            "monitor-temp",
            t!("monitor_temp_unit", unit = temp_unit_name).into_owned(),
            hottest.map_or("—".to_string(), |v| settings.temp(v)),
            chart::area2(
                "monitor-temp-chart",
                chart::series2(s, None, move |t| {
                    let hot = t
                        .max_temperature()
                        .map_or(0.0, |v| fmt::temp_chart_value(v, temp_unit));
                    let cpu = t
                        .cpu_temperature()
                        .map_or(hot, |v| fmt::temp_chart_value(v, temp_unit));
                    (hot, cpu)
                }),
                (palette[4], palette[0]),
                (
                    t!("monitor_hottest").into_owned(),
                    t!("dash_card_cpu").into_owned(),
                ),
                |v| format!("{v:.0}"),
                cx,
            ),
            cx,
        )
    } else {
        ui::section(
            "monitor-temp",
            t!("monitor_temp_unit", unit = temp_unit_name).into_owned(),
        )
        .flex_1()
        .min_w(rems(26.))
        .child(ui::muted(t!("monitor_no_temp").into_owned(), cx))
        .into_any_element()
    });

    // GPU：最多画两块。
    let gpu_names: Vec<String> = s
        .snapshot
        .gpus
        .iter()
        .filter(|g| g.utilization.is_some())
        .take(2)
        .map(|g| g.name.clone())
        .collect();
    if let Some(first) = gpu_names.first().cloned() {
        let second = gpu_names.get(1).cloned();
        let util_of = |tick: &pc_specs_core::model::MonitorTick, name: &str| {
            tick.gpus
                .iter()
                .find(|g| g.name == name)
                .and_then(|g| g.utilization)
                .unwrap_or(0.0) as f64
        };
        let value = latest
            .and_then(|t| {
                t.gpus
                    .iter()
                    .find(|g| g.name == first)
                    .and_then(|g| g.utilization)
            })
            .map_or("—".to_string(), fmt::percent);
        let body = match second {
            Some(second) => {
                let (n1, n2) = (first.clone(), second.clone());
                chart::area2(
                    "monitor-gpu-chart",
                    chart::series2(s, None, move |t| (util_of(t, &n1), util_of(t, &n2))),
                    (palette[3], palette[1]),
                    (first, second),
                    |v| format!("{v:.0}%"),
                    cx,
                )
            }
            None => {
                let n1 = first.clone();
                chart::area(
                    "monitor-gpu-chart",
                    chart::series(s, None, move |t| util_of(t, &n1)),
                    Some((0.0, 100.0)),
                    palette[3],
                    |v| format!("{v:.0}%"),
                )
            }
        };
        cards.push(chart_card(
            "monitor-gpu",
            t!("monitor_gpu").into_owned(),
            value,
            body,
            cx,
        ));
    }

    let charts = h_flex()
        .w_full()
        .flex_wrap()
        .items_stretch()
        .gap_4()
        .children(cards);

    // ---- 每核心 ----
    let per_core = latest.map(|t| t.cpu_per_core.clone()).unwrap_or_default();
    let cores = ui::section("monitor-cores", t!("monitor_per_core_latest").into_owned()).child(
        if per_core.is_empty() {
            ui::muted(t!("monitor_no_data").into_owned(), cx).into_any_element()
        } else {
            h_flex()
                .flex_wrap()
                .gap_3()
                .children(per_core.iter().enumerate().map(|(i, usage)| {
                    v_flex()
                        .w(rems(7.))
                        .gap_1()
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .child(ui::dim(format!("#{i}"), cx))
                                .child(div().child(fmt::percent(*usage))),
                        )
                        .child(
                            ui::usage_bar(
                                SharedString::from(format!("monitor-core-{i}")),
                                *usage,
                                cx,
                            )
                            .small(),
                        )
                        .into_any_element()
                }))
                .into_any_element()
        },
    );

    ui::page(
        "page-monitor",
        t!("monitor_title").into_owned(),
        s.paused
            .then(|| SharedString::from(t!("monitor_paused").into_owned())),
        vec![range, pause, clear],
        vec![charts.into_any_element(), cores.into_any_element()],
        cx,
    )
}

fn chart_card(
    id: &'static str,
    title: String,
    value: String,
    chart: AnyElement,
    _cx: &App,
) -> AnyElement {
    ui::section(id, title)
        .flex_1()
        .min_w(rems(26.))
        .child(
            v_flex()
                .gap_2()
                .child(div().text_lg().font_semibold().child(value))
                .child(div().h(rems(CHART_HEIGHT)).w_full().child(chart)),
        )
        .into_any_element()
}
