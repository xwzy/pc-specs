//! 主板与 BIOS / UEFI。

use crate::state::AppState;
use crate::ui;
use gpui_kit::assets::IconName;
use gpui_kit::{AnyElement, App, IntoElement as _, ParentElement as _};
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let body = match &s.snapshot.motherboard {
        None => vec![ui::empty_state(
            IconName::CircuitBoard,
            t!("mb_no_data").into_owned(),
            t!("mb_no_data_hint").into_owned(),
        )],
        Some(mb) => {
            let text = |v: &Option<String>| v.clone().unwrap_or_default();
            let items = vec![
                (t!("spec_vendor").into_owned(), text(&mb.vendor)),
                (t!("spec_brand").into_owned(), text(&mb.model)),
                (t!("mb_board_version").into_owned(), text(&mb.version)),
                (t!("spec_serial").into_owned(), text(&mb.serial)),
                (t!("mb_bios_vendor").into_owned(), text(&mb.bios_vendor)),
                (t!("mb_bios_version").into_owned(), text(&mb.bios_version)),
                (t!("mb_bios_date").into_owned(), text(&mb.bios_date)),
                (t!("mb_chassis").into_owned(), text(&mb.chassis)),
            ];
            vec![
                ui::section("mb-details", t!("cpu_details").into_owned())
                    .child(ui::spec_list(items))
                    .into_any_element(),
            ]
        }
    };
    ui::page(
        "page-motherboard",
        t!("nav_motherboard").into_owned(),
        None,
        vec![],
        body,
        cx,
    )
}
