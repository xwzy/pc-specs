//! 外设：USB / 蓝牙 / 输入设备等。

use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::{AnyElement, App, IntoElement as _, SharedString};
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let list = &s.snapshot.peripherals;
    let (subtitle, body) = if list.is_empty() {
        (
            None,
            vec![ui::empty_state(
                IconName::Usb,
                t!("peri_no_devices").into_owned(),
                t!("peri_no_devices_hint").into_owned(),
            )],
        )
    } else {
        // 只有读到了电量的设备才有这一列，否则整列都是「—」。
        let has_battery = list.iter().any(|p| p.battery_percent.is_some());
        let mut cols = vec![
            ui::Col::new(t!("peri_kind").into_owned()).width(8.),
            ui::Col::new(t!("peri_name").into_owned()),
            ui::Col::new(t!("peri_ids").into_owned()).width(11.),
            ui::Col::new(t!("peri_bus").into_owned()).width(8.),
        ];
        if has_battery {
            cols.push(ui::Col::new(t!("peri_battery").into_owned()).width(6.));
        }
        let rows = list
            .iter()
            .map(|p| {
                let mut row = vec![
                    ui::chip(kind_label(&p.kind)).into_any_element(),
                    ui::txt(p.name.clone()),
                    ui::txt(fmt::or_dash(
                        &[p.vendor_id.as_deref(), p.product_id.as_deref()]
                            .into_iter()
                            .flatten()
                            .filter(|id| !id.is_empty())
                            .collect::<Vec<_>>()
                            .join(" : "),
                    )),
                    ui::txt(fmt::or_dash(p.bus.as_deref().unwrap_or(""))),
                ];
                if has_battery {
                    row.push(ui::txt(p.battery_percent.map_or_else(
                        || "—".to_string(),
                        |percent| format!("{percent}%"),
                    )));
                }
                row
            })
            .collect();
        (
            Some(SharedString::from(
                t!("count_found", n = list.len()).into_owned(),
            )),
            vec![ui::table(cols, rows).into_any_element()],
        )
    };
    ui::page(
        "page-peripherals",
        t!("nav_peripherals").into_owned(),
        subtitle,
        vec![],
        body,
        cx,
    )
}

/// 设备类别的显示名；采集端给的是 `usb` / `bluetooth` 这样的英文小写标识。
fn kind_label(kind: &str) -> String {
    match kind {
        "usb" => "USB".to_string(),
        "bluetooth" => t!("peri_kind_bluetooth").into_owned(),
        "camera" => t!("peri_kind_camera").into_owned(),
        "audio" => t!("peri_kind_audio").into_owned(),
        "printer" => t!("peri_kind_printer").into_owned(),
        other => other.to_uppercase(),
    }
}
