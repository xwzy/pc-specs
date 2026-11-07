//! 电池：电量、状态、健康度与容量。台式机没有电池时给出空状态。

use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, progress::Progress, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
};
use rust_i18n::t;

fn state_label(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "charging" => t!("bat_state_charging").into_owned(),
        "discharging" => t!("bat_state_discharging").into_owned(),
        "full" => t!("bat_state_full").into_owned(),
        "empty" => t!("bat_state_empty").into_owned(),
        "unknown" => t!("bat_state_unknown").into_owned(),
        _ => raw.to_string(),
    }
}

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let Some(bat) = &s.snapshot.battery else {
        return ui::page(
            "page-battery",
            t!("nav_battery").into_owned(),
            None,
            vec![],
            vec![ui::empty_state(
                IconName::BatteryWarning,
                t!("bat_no_battery").into_owned(),
                t!("bat_no_battery_hint").into_owned(),
            )],
            cx,
        );
    };

    let state = state_label(&bat.state);
    // 电量越低越危险，与占用率的配色方向相反。
    let color = if bat.percentage < 15.0 {
        cx.theme().danger
    } else if bat.percentage < 30.0 {
        cx.theme().warning
    } else {
        cx.theme().success
    };

    // 健康度 = 满电容量 / 设计容量。
    let health = match (bat.full_capacity_mwh, bat.design_capacity_mwh) {
        (Some(full), Some(design)) if design > 0 => Some(full as f32 / design as f32 * 100.0),
        _ => None,
    };

    let mwh = |v: Option<u64>| v.map_or(String::new(), fmt::watt_hours);
    let minutes = |v: Option<u64>| v.filter(|v| *v > 0).map_or(String::new(), fmt::duration);

    let items = vec![
        (
            t!("spec_vendor").into_owned(),
            bat.vendor.clone().unwrap_or_default(),
        ),
        (
            t!("spec_brand").into_owned(),
            bat.model.clone().unwrap_or_default(),
        ),
        (t!("spec_status").into_owned(), state.clone()),
        (
            t!("bat_cycle").into_owned(),
            bat.cycle_count.map_or(String::new(), |c| c.to_string()),
        ),
        (
            t!("bat_health").into_owned(),
            health.map_or(String::new(), fmt::percent),
        ),
        (
            t!("bat_design_cap").into_owned(),
            mwh(bat.design_capacity_mwh),
        ),
        (t!("bat_full_cap").into_owned(), mwh(bat.full_capacity_mwh)),
        (
            t!("bat_curr_cap").into_owned(),
            mwh(bat.current_capacity_mwh),
        ),
        (
            t!("spec_temp").into_owned(),
            bat.temperature_c
                .map_or(String::new(), |v| s.settings.temp(v)),
        ),
        (
            t!("bat_power").into_owned(),
            bat.power_now_mw
                .map_or(String::new(), |mw| format!("{:.2} W", mw as f64 / 1000.0)),
        ),
        (
            t!("bat_time_to_empty").into_owned(),
            minutes(bat.time_to_empty_secs),
        ),
        (
            t!("bat_time_to_full").into_owned(),
            minutes(bat.time_to_full_secs),
        ),
    ];

    let overview = ui::section("bat-overview", t!("nav_dashboard").into_owned()).child(
        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .items_baseline()
                    .gap_3()
                    .child(
                        div()
                            .text_3xl()
                            .font_semibold()
                            .child(fmt::percent(bat.percentage)),
                    )
                    .child(ui::muted(state.clone(), cx)),
            )
            .child(Progress::new("bat-bar").value(bat.percentage).color(color))
            .child(ui::spec_list(items)),
    );

    ui::page(
        "page-battery",
        t!("nav_battery").into_owned(),
        Some(SharedString::from(state)),
        vec![],
        vec![overview.into_any_element()],
        cx,
    )
}
