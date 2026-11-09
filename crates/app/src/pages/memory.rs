//! 内存：占用环形图、容量统计、实时走势与内存条表格。

use super::chart;
use crate::health::Level;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _, h_flex, progress::ProgressCircle, v_flex,
};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
    prelude::FluentBuilder as _, rems,
};
use pc_specs_core::model::MemoryBreakdown;
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let mem = &s.snapshot.memory;
    let settings = &s.settings;
    let palette = chart::palette(cx);

    // 实时值优先，快照里的值只在第一个采样到达前使用。
    let (used, total, swap_used, swap_total) = s
        .latest
        .as_ref()
        .map(|t| {
            (
                t.mem_used_bytes,
                t.mem_total_bytes,
                t.swap_used_bytes,
                t.swap_total_bytes,
            )
        })
        .unwrap_or((
            mem.used_bytes,
            mem.total_bytes,
            mem.swap_used_bytes,
            mem.swap_total_bytes,
        ));
    let used_pct = fmt::ratio(used, total);
    let swap_pct = fmt::ratio(swap_used, swap_total);

    let stat = |label: String, value: String| {
        v_flex()
            .w(rems(9.))
            .gap_0p5()
            .child(ui::muted(label, cx).text_xs())
            .child(div().font_semibold().child(value))
    };

    let overview = ui::section("mem-overview", t!("mem_usage").into_owned()).child(
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_6()
                    .flex_wrap()
                    .items_center()
                    .child(
                        ProgressCircle::new("mem-ring")
                            .value(used_pct)
                            .color(ui::usage_color(used_pct, cx))
                            .size(rems(7.))
                            .child(
                                v_flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_semibold()
                                            .child(fmt::percent(used_pct)),
                                    )
                                    .child(ui::muted(t!("spec_used").into_owned(), cx).text_xs()),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w(rems(14.))
                            .flex_wrap()
                            .gap_x_6()
                            .gap_y_3()
                            .child(stat(t!("spec_total").into_owned(), settings.bytes(total)))
                            .child(stat(t!("spec_used").into_owned(), settings.bytes(used)))
                            .child(stat(
                                t!("spec_available").into_owned(),
                                settings.bytes(total.saturating_sub(used)),
                            )),
                    ),
            )
            // 交换空间只在系统启用了它时才有意义，没有就不占位。
            .when(swap_total > 0, |this| {
                this.child(ui::meter(
                    "mem-swap-bar",
                    t!("mem_swap").into_owned(),
                    format!(
                        "{} / {}",
                        settings.bytes(swap_used),
                        settings.bytes(swap_total)
                    ),
                    swap_pct,
                    cx,
                ))
            }),
    );

    let composition = s
        .latest
        .as_ref()
        .and_then(|t| t.memory.as_ref())
        .filter(|m| m.app_bytes + m.wired_bytes + m.compressed_bytes + m.cached_bytes > 0)
        .map(|m| composition(m, s, cx));

    let live = ui::section("mem-live", t!("mem_usage_live").into_owned()).child(
        div().h(rems(10.)).w_full().child(chart::area(
            "mem-live-chart",
            chart::series(s, Some(120), |t| {
                fmt::ratio(t.mem_used_bytes, t.mem_total_bytes) as f64
            }),
            Some((0.0, 100.0)),
            palette[1],
            |v| format!("{v:.0}%"),
            "%",
        )),
    );

    let modules = ui::plain_section("mem-modules", t!("mem_modules").into_owned()).child(
        if mem.modules.is_empty() {
            ui::empty_state(
                IconName::MemoryStick,
                t!("mem_no_modules").into_owned(),
                t!("mem_no_modules_hint").into_owned(),
            )
        } else {
            let cols = vec![
                ui::Col::new(t!("mem_table_slot").into_owned()),
                ui::Col::new(t!("mem_table_manufacturer").into_owned()),
                ui::Col::new(t!("mem_table_part").into_owned()),
                ui::Col::new(t!("mem_table_capacity").into_owned()).right(),
                ui::Col::new(t!("mem_table_speed").into_owned()).right(),
                ui::Col::new(t!("mem_table_type").into_owned()),
                ui::Col::new(t!("mem_table_form").into_owned()),
            ];
            let rows = mem
                .modules
                .iter()
                .map(|m| {
                    vec![
                        ui::txt(module_text(&m.slot)),
                        ui::txt(fmt::or_dash(m.manufacturer.as_deref().unwrap_or(""))),
                        ui::txt(fmt::or_dash(m.part_number.as_deref().unwrap_or(""))),
                        ui::txt(settings.bytes(m.capacity_bytes)),
                        ui::txt(
                            m.speed_mt_s
                                .map_or("—".to_string(), |v| format!("{v} MT/s")),
                        ),
                        ui::txt(fmt::or_dash(m.kind.as_deref().unwrap_or(""))),
                        ui::txt(fmt::or_dash(&module_text(
                            m.form_factor.as_deref().unwrap_or(""),
                        ))),
                    ]
                })
                .collect();
            ui::table(cols, rows).into_any_element()
        },
    );

    ui::page(
        "page-memory",
        t!("nav_memory").into_owned(),
        Some(SharedString::from(settings.bytes(total))),
        vec![],
        [overview.into_any_element()]
            .into_iter()
            .chain(composition)
            .chain([live.into_any_element(), modules.into_any_element()])
            .collect(),
        cx,
    )
}

/// 内存构成：一条分段占比条、图例，以及系统给出时的内存压力和换页速率。
fn composition(memory: &MemoryBreakdown, s: &AppState, cx: &App) -> AnyElement {
    let settings = &s.settings;
    let [blue, green, _, magenta, yellow] = chart::palette(cx);
    let parts = [
        (t!("mem_app"), memory.app_bytes, blue),
        (t!("mem_wired"), memory.wired_bytes, yellow),
        (t!("mem_compressed"), memory.compressed_bytes, magenta),
        (t!("mem_cached"), memory.cached_bytes, green),
        (
            t!("mem_free"),
            memory.free_bytes,
            cx.theme().muted_foreground.opacity(0.35),
        ),
    ];
    let total: u64 = parts.iter().map(|p| p.1).sum();
    let bar: Vec<_> = parts.iter().map(|(_, n, color)| (*color, *n)).collect();
    let legend = h_flex().flex_wrap().gap_x_6().gap_y_2().children(
        parts
            .iter()
            .filter(|(_, n, _)| *n > 0)
            .map(|(name, n, color)| {
                ui::legend_item(
                    *color,
                    name.to_string(),
                    format!(
                        "{} · {}",
                        settings.bytes(*n),
                        fmt::percent(fmt::ratio(*n, total))
                    ),
                    cx,
                )
            }),
    );

    let stat = |label: String, value: AnyElement| {
        v_flex()
            .w(rems(9.))
            .gap_0p5()
            .child(ui::muted(label, cx).text_xs())
            .child(value)
    };
    let mut stats = h_flex().flex_wrap().gap_x_6().gap_y_3();
    if let Some(percent) = memory.pressure_percent {
        let (level, name) = match memory.pressure_level {
            Some(2) => (Level::Critical, t!("mem_pressure_critical")),
            Some(1) => (Level::Warning, t!("mem_pressure_warning")),
            _ => (Level::Healthy, t!("mem_pressure_normal")),
        };
        stats = stats.child(stat(
            t!("mem_pressure").into_owned(),
            h_flex()
                .items_center()
                .gap_2()
                .child(ui::level_tag(level, name.into_owned()))
                .child(ui::muted(fmt::percent(percent), cx))
                .into_any_element(),
        ));
    }
    if memory.pressure_percent.is_some() || memory.swapped_in_bytes + memory.swapped_out_bytes > 0 {
        let rate = |bps: u64| {
            div()
                .font_semibold()
                .child(format!("{}/s", settings.bytes(bps)))
                .into_any_element()
        };
        stats = stats
            .child(stat(
                t!("mem_swap_in").into_owned(),
                rate(memory.swap_in_bps),
            ))
            .child(stat(
                t!("mem_swap_out").into_owned(),
                rate(memory.swap_out_bps),
            ));
    }

    ui::section("mem-composition", t!("mem_composition").into_owned())
        .child(
            v_flex()
                .gap_3()
                .child(ui::stacked_bar(&bar, cx))
                .child(legend)
                .child(stats)
                .child(ui::muted(t!("mem_composition_hint").into_owned(), cx).text_xs()),
        )
        .into_any_element()
}

/// 采集端给的槽位 / 形态是英文原文；其中「板载、焊接」是 Apple 芯片上固定的两个值，翻译成界面语言。
fn module_text(raw: &str) -> String {
    match raw {
        "Onboard" => t!("mem_onboard").into_owned(),
        "Soldered" => t!("mem_soldered").into_owned(),
        other => other.to_string(),
    }
}
