//! 传感器：温度 / 风扇 / 电压 / 功率 / 电流，读数随实时采样刷新。

use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, div, prelude::FluentBuilder as _,
};
use pc_specs_core::model::SensorReading;
use pc_specs_core::modules::{cpu::cpu_temperature, gpu::gpu_temperature};
use rust_i18n::t;

/// 展示顺序。未列出的类别归入「其他」。
const KINDS: [&str; 5] = ["temperature", "fan", "voltage", "power", "current"];

/// 一张读数卡片的最小宽度（px）。
const TILE_MIN_WIDTH: f32 = 200.0;

/// 这几个「来源」说的是用什么方式读到的，不是哪块硬件，卡片上标出来只是噪音。
const GENERIC_SOURCES: [&str; 2] = ["sysinfo", "smc"];

pub fn render(s: &AppState, content_width: f32, cx: &App) -> AnyElement {
    // 温度、风扇、功率这些随实时采样持续刷新；平台没有实时读数的类别（比如 Windows 的热区）退回快照里的值。
    let live = s.latest.as_ref();
    let live_temps = live
        .map(|t| t.temperatures.as_slice())
        .filter(|t| !t.is_empty());
    let live_others = live
        .map(|t| t.other_sensors.as_slice())
        .filter(|t| !t.is_empty());
    let mut readings: Vec<&SensorReading> = s
        .snapshot
        .sensors
        .iter()
        .filter(|r| {
            let live = if r.kind == "temperature" {
                live_temps
            } else {
                live_others
            };
            live.is_none()
        })
        .collect();
    readings.extend(live_temps.unwrap_or_default());
    readings.extend(live_others.unwrap_or_default());
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

    // 所有读数都来自同一个来源时，每张卡片再标一遍就是噪音，只在来源不一样时才标。
    let mixed_sources = readings.iter().any(|r| r.source != readings[0].source);
    // 区块左右还有内边距，网格可用的宽度要比页面正文窄一点。
    let cols = ui::columns(content_width - 40.0, TILE_MIN_WIDTH, 6);

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
        let tiles = div().w_full().grid().grid_cols(cols).gap_4().children(
            group
                .iter()
                .enumerate()
                .map(|(i, r)| tile(kind, i, r, mixed_sources, s, cx)),
        );
        body.push(
            ui::section(
                SharedString::from(format!("sensors-{kind}")),
                format!("{title} ({})", group.len()),
            )
            .child(
                v_flex()
                    .gap_3()
                    .when(kind == "temperature", |this| {
                        this.when_some(temperature_summary(&group, s), |this, text| {
                            this.child(ui::muted(text, cx))
                        })
                    })
                    .child(tiles),
            )
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

/// 温度区块顶上的一句话：最热的是谁，CPU、GPU 各是多少，不用在几十张卡片里自己找。
fn temperature_summary(group: &[&SensorReading], s: &AppState) -> Option<String> {
    let valid: Vec<SensorReading> = group
        .iter()
        .filter(|r| r.value.is_finite())
        .map(|r| (*r).clone())
        .collect();
    if valid.len() < 2 {
        return None;
    }
    let hottest = valid.iter().max_by(|a, b| a.value.total_cmp(&b.value))?;
    let mut parts = vec![
        t!(
            "sensors_summary",
            max = s.settings.temp(hottest.value),
            name = fmt::sensor_label(hottest)
        )
        .into_owned(),
    ];
    if let Some(cpu) = cpu_temperature(&valid) {
        parts.push(t!("sensors_summary_cpu", value = s.settings.temp(cpu)).into_owned());
    }
    if let Some(gpu) = gpu_temperature(&valid) {
        parts.push(t!("sensors_summary_gpu", value = s.settings.temp(gpu)).into_owned());
    }
    if let Some(pressure) = s.latest.as_ref().and_then(|t| t.thermal_pressure) {
        parts.push(
            t!(
                "sensors_summary_thermal",
                level = fmt::thermal_pressure(pressure)
            )
            .into_owned(),
        );
    }
    Some(parts.join(" · "))
}

fn tile(
    kind: &str,
    index: usize,
    r: &SensorReading,
    show_source: bool,
    s: &AppState,
    cx: &App,
) -> AnyElement {
    let (value, color) = match r.unit.as_str() {
        "C" => {
            let level = crate::health::temperature_level(r.value);
            let color = if level == crate::health::Level::Healthy {
                cx.theme().foreground
            } else {
                ui::level_color(level, cx)
            };
            (s.settings.temp(r.value), color)
        }
        _ => (fmt::sensor_value(r), cx.theme().foreground),
    };
    v_flex()
        .id(SharedString::from(format!("sensor-{kind}-{index}")))
        .min_w_0()
        .gap_1()
        .p_3()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .when(
            show_source && !GENERIC_SOURCES.contains(&r.source.as_str()),
            |this| this.child(h_flex().child(ui::chip(fmt::or_dash(&r.source)))),
        )
        .child(ui::muted(fmt::sensor_label(r), cx).text_xs().truncate())
        .child(
            div()
                .text_xl()
                .font_semibold()
                .text_color(color)
                .child(value),
        )
        .into_any_element()
}
