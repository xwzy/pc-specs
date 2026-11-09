//! 处理器：实时占用、每核心占用、规格与（极客模式下的）拓扑 / 指令集。

use super::chart;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div, rems,
};
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let cpu = &s.snapshot.cpu;
    let settings = &s.settings;
    let palette = chart::palette(cx);

    let overall = s
        .latest
        .as_ref()
        .map_or(cpu.usage_overall, |t| t.cpu_overall);
    let per_core: Vec<f32> = s
        .latest
        .as_ref()
        .map(|t| t.cpu_per_core.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| cpu.usage_per_core.clone());

    // ---- 实时占用 ----
    let load = s.latest.as_ref().and_then(|t| t.load_average);
    let stat = |label: String, value: String| {
        v_flex()
            .gap_0p5()
            .child(ui::muted(label, cx).text_xs())
            .child(div().font_semibold().child(value))
    };
    let temperature = s
        .latest
        .as_ref()
        .and_then(|t| t.cpu_temperature())
        .or(cpu.temperature_c);
    let live = ui::section("cpu-live", t!("cpu_usage_live").into_owned()).child(
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .items_baseline()
                    .gap_3()
                    .child(
                        div()
                            .text_3xl()
                            .font_semibold()
                            .child(fmt::percent(overall)),
                    )
                    .child(ui::muted(
                        temperature.map_or(String::new(), |v| settings.temp(v)),
                        cx,
                    )),
            )
            .child(ui::usage_bar("cpu-overall-bar", overall, cx))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_x_8()
                    .gap_y_2()
                    .children(load.map(|[one, five, fifteen]| {
                        stat(
                            t!("cpu_load_average").into_owned(),
                            format!("{one:.2} · {five:.2} · {fifteen:.2}"),
                        )
                    }))
                    .children(
                        s.uptime_secs()
                            .map(|secs| stat(t!("spec_uptime").into_owned(), fmt::duration(secs))),
                    ),
            )
            .child(div().h(rems(9.)).w_full().child(chart::area(
                "cpu-live-chart",
                chart::series(s, Some(120), |t| t.cpu_overall as f64),
                Some((0.0, 100.0)),
                palette[0],
                |v| format!("{v:.0}%"),
                "%",
            ))),
    );

    // ---- 每核心 ----
    let cores = ui::section("cpu-cores", t!("cpu_per_core").into_owned()).child(
        h_flex()
            .w_full()
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
                        ui::usage_bar(SharedString::from(format!("core-bar-{i}")), *usage, cx)
                            .small(),
                    )
                    .into_any_element()
            })),
    );

    // ---- 规格 ----
    let mut items = vec![
        (t!("spec_vendor").into_owned(), cpu.vendor.clone()),
        (t!("spec_brand").into_owned(), cpu.brand.clone()),
        (t!("spec_arch").into_owned(), cpu.arch.clone()),
        (t!("spec_cores").into_owned(), fmt::core_summary(cpu)),
        (
            t!("spec_freq").into_owned(),
            fmt::freq(cpu.current_frequency_hz),
        ),
        (
            t!("spec_freq_max").into_owned(),
            fmt::freq(cpu.max_frequency_hz),
        ),
        (
            t!("cpu_cache_l1").into_owned(),
            cpu.cache_l1_bytes
                .map_or(String::new(), |b| settings.bytes(b)),
        ),
        (
            t!("cpu_cache_l2").into_owned(),
            cpu.cache_l2_bytes
                .map_or(String::new(), |b| settings.bytes(b)),
        ),
        (
            t!("cpu_cache_l3").into_owned(),
            cpu.cache_l3_bytes
                .map_or(String::new(), |b| settings.bytes(b)),
        ),
        (
            t!("cpu_virtualization").into_owned(),
            cpu.virtualization.map_or(String::new(), |v| {
                if v {
                    t!("common_yes").into_owned()
                } else {
                    t!("common_no").into_owned()
                }
            }),
        ),
    ];
    if let Some(temp) = temperature {
        items.push((t!("spec_temp").into_owned(), settings.temp(temp)));
    }
    let details = ui::plain_section("cpu-details", t!("cpu_details").into_owned())
        .child(ui::spec_list(items, cx));

    let mut body = vec![
        live.into_any_element(),
        cores.into_any_element(),
        details.into_any_element(),
    ];

    // ---- 极客面板 ----
    if settings.geek_mode {
        let mut geek = v_flex().gap_3();
        if let Some(t) = &cpu.topology {
            let mut topo = vec![
                (t!("cpu_sockets").into_owned(), t.sockets.to_string()),
                (t!("cpu_numa").into_owned(), t.numa_nodes.to_string()),
            ];
            if let Some(p) = t.p_cores {
                topo.push((t!("cpu_p_cores").into_owned(), p.to_string()));
            }
            if let Some(e) = t.e_cores {
                topo.push((t!("cpu_e_cores").into_owned(), e.to_string()));
            }
            geek = geek.child(ui::spec_list(topo, cx));
        }
        geek = geek.child(
            v_flex()
                .gap_2()
                .child(ui::muted(t!("cpu_isa_features").into_owned(), cx))
                .child(if cpu.features.is_empty() {
                    ui::muted(t!("cpu_no_features").into_owned(), cx).into_any_element()
                } else {
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .children(
                            cpu.features
                                .iter()
                                .map(|f| ui::chip(f.clone()).into_any_element()),
                        )
                        .into_any_element()
                }),
        );
        body.push(
            ui::section("cpu-geek", t!("cpu_geek_panel").into_owned())
                .child(geek)
                .into_any_element(),
        );
    }

    ui::page(
        "page-cpu",
        t!("nav_cpu").into_owned(),
        (!cpu.brand.is_empty()).then(|| SharedString::from(cpu.brand.clone())),
        vec![],
        body,
        cx,
    )
}
