//! 内存：占用环形图、容量统计、实时走势与内存条表格。

use super::chart;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{StyledExt as _, h_flex, progress::ProgressCircle, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div, rems,
};
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

    let overview = ui::section("mem-overview", t!("nav_dashboard").into_owned()).child(
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
                            .min_w(rems(18.))
                            .flex_wrap()
                            .gap_x_6()
                            .gap_y_3()
                            .child(stat(t!("spec_total").into_owned(), settings.bytes(total)))
                            .child(stat(t!("spec_used").into_owned(), settings.bytes(used)))
                            .child(stat(
                                t!("spec_available").into_owned(),
                                settings.bytes(total.saturating_sub(used)),
                            ))
                            .child(stat(
                                t!("mem_swap_total").into_owned(),
                                settings.bytes(swap_total),
                            ))
                            .child(stat(
                                t!("mem_swap_used").into_owned(),
                                settings.bytes(swap_used),
                            ))
                            .child(stat(
                                t!("mem_swap_pct").into_owned(),
                                fmt::percent(swap_pct),
                            )),
                    ),
            )
            .child(ui::meter(
                "mem-ram-bar",
                "RAM",
                format!("{} / {}", settings.bytes(used), settings.bytes(total)),
                used_pct,
                cx,
            ))
            .child(ui::meter(
                "mem-swap-bar",
                t!("mem_swap").into_owned(),
                format!(
                    "{} / {}",
                    settings.bytes(swap_used),
                    settings.bytes(swap_total)
                ),
                swap_pct,
                cx,
            )),
    );

    let live = ui::section("mem-live", t!("mem_usage_live").into_owned()).child(
        div().h(rems(10.)).w_full().child(chart::area(
            "mem-live-chart",
            chart::series(s, Some(120), |t| {
                fmt::ratio(t.mem_used_bytes, t.mem_total_bytes) as f64
            }),
            Some((0.0, 100.0)),
            palette[1],
            |v| format!("{v:.0}%"),
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
                        ui::txt(m.slot.clone()),
                        ui::txt(fmt::or_dash(m.manufacturer.as_deref().unwrap_or(""))),
                        ui::txt(fmt::or_dash(m.part_number.as_deref().unwrap_or(""))),
                        ui::txt(settings.bytes(m.capacity_bytes)),
                        ui::txt(
                            m.speed_mt_s
                                .map_or("—".to_string(), |v| format!("{v} MT/s")),
                        ),
                        ui::txt(fmt::or_dash(m.kind.as_deref().unwrap_or(""))),
                        ui::txt(fmt::or_dash(m.form_factor.as_deref().unwrap_or(""))),
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
        vec![
            overview.into_any_element(),
            live.into_any_element(),
            modules.into_any_element(),
        ],
        cx,
    )
}
