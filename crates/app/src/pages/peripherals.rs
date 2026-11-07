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
        let cols = vec![
            ui::Col::new(t!("peri_kind").into_owned()).width(8.),
            ui::Col::new(t!("peri_name").into_owned()),
            ui::Col::new(t!("peri_ids").into_owned()).width(11.),
            ui::Col::new(t!("peri_bus").into_owned()).width(8.),
        ];
        let rows = list
            .iter()
            .map(|p| {
                vec![
                    ui::chip(p.kind.to_uppercase()).into_any_element(),
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
                ]
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
