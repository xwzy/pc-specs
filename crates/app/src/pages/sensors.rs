//! 传感器：温度 / 风扇 / 电压 / 功率 / 电流，温度读数随实时采样刷新。

use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, div, rems,
};
use pc_specs_core::model::SensorReading;
use rust_i18n::t;

/// 展示顺序。未列出的类别归入「其他」。
const KINDS: [&str; 5] = ["temperature", "fan", "voltage", "power", "current"];

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    // 温度用最近一次实时采样（持续刷新），其余类别用快照里的读数。
    let live_temps: Option<&Vec<SensorReading>> = s
        .latest
        .as_ref()
        .map(|t| &t.temperatures)
        .filter(|t| !t.is_empty());
    let mut readings: Vec<&SensorReading> = s
        .snapshot
        .sensors
        .iter()
        .filter(|r| r.kind != "temperature" || live_temps.is_none())
        .collect();
    if let Some(temps) = live_temps {
        readings.extend(temps.iter());
    }
    // 按名称排序，实时刷新时卡片位置才不会跳。
    readings.sort_by(|a, b| fmt::natural_cmp(&a.label, &b.label));

    if readings.is_empty() {
        return ui::page(
            "page-sensors",
            t!("nav_sensors").into_owned(),
            None,
            vec![],
            vec![ui::empty_state(
                IconName::Thermometer,
                t!("sensors_no_data").into_owned(),
                t!("sensors_no_data_hint").into_owned(),
            )],
            cx,
        );
    }

    let mut body = Vec::new();
    for kind in KINDS.iter().copied().chain(std::iter::once("other")) {
        let group: Vec<&SensorReading> = readings
            .iter()
            .copied()
            .filter(|r| {
                if kind == "other" {
                    !KINDS.contains(&r.kind.as_str())
                } else {
                    r.kind == kind
                }
            })
            .collect();
        if group.is_empty() {
            continue;
        }
        let title = match kind {
            "temperature" => t!("sensors_temperature"),
            "fan" => t!("sensors_fan"),
            "voltage" => t!("sensors_voltage"),
            "power" => t!("sensors_power"),
            "current" => t!("sensors_current"),
            _ => t!("sensors_other"),
        };
        let tiles = h_flex().flex_wrap().gap_3().children(
            group
                .iter()
                .enumerate()
                .map(|(i, r)| tile(kind, i, r, s, cx)),
        );
        body.push(
            ui::section(
                SharedString::from(format!("sensors-{kind}")),
                format!("{title} ({})", group.len()),
            )
            .child(tiles)
            .into_any_element(),
        );
    }

    ui::page(
        "page-sensors",
        t!("nav_sensors").into_owned(),
        Some(SharedString::from(
            t!("count_found", n = readings.len()).into_owned(),
        )),
        vec![],
        body,
        cx,
    )
}

fn tile(kind: &str, index: usize, r: &SensorReading, s: &AppState, cx: &App) -> AnyElement {
    let (value, color) = match r.unit.as_str() {
        "C" => {
            let level = if r.value >= 88.0 {
                crate::health::Level::Critical
            } else if r.value >= 75.0 {
                crate::health::Level::Warning
            } else {
                crate::health::Level::Healthy
            };
            let color = if level == crate::health::Level::Healthy {
                cx.theme().foreground
            } else {
                ui::level_color(level, cx)
            };
            (s.settings.temp(r.value), color)
        }
        "RPM" => (format!("{:.0} RPM", r.value), cx.theme().foreground),
        "V" => (format!("{:.2} V", r.value), cx.theme().foreground),
        "W" => (format!("{:.1} W", r.value), cx.theme().foreground),
        unit => (format!("{:.1} {unit}", r.value), cx.theme().foreground),
    };
    v_flex()
        .id(SharedString::from(format!("sensor-{kind}-{index}")))
        .w(rems(13.))
        .gap_1()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .child(h_flex().child(ui::chip(fmt::or_dash(&r.source))))
        .child(ui::muted(r.label.clone(), cx).text_xs().truncate())
        .child(
            div()
                .text_xl()
                .font_semibold()
                .text_color(color)
                .child(value),
        )
        .into_any_element()
}
