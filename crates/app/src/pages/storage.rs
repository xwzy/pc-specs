//! 存储：每个卷一张卡片，含容量占用、实时读写与 SMART 状态。

use crate::health::Level;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
    prelude::FluentBuilder as _,
};
use pc_specs_core::model::StorageInfo;
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let disks = &s.snapshot.storages;
    let (subtitle, body) = if disks.is_empty() {
        (
            None,
            vec![ui::empty_state(
                IconName::HardDrive,
                t!("storage_no_disks").into_owned(),
                "",
            )],
        )
    } else {
        (
            Some(SharedString::from(
                t!("count_found", n = disks.len()).into_owned(),
            )),
            disks
                .iter()
                .enumerate()
                .map(|(i, d)| card(i, d, s, cx))
                .collect(),
        )
    };
    ui::page(
        "page-storage",
        t!("nav_storage").into_owned(),
        subtitle,
        vec![],
        body,
        cx,
    )
}

fn card(index: usize, d: &StorageInfo, s: &AppState, cx: &App) -> AnyElement {
    let settings = &s.settings;
    let pct = fmt::ratio(d.used_bytes, d.total_bytes);

    let smart = d.smart_health.as_ref().map(|state| {
        let ok = state.eq_ignore_ascii_case("ok") || state.eq_ignore_ascii_case("verified");
        let text = if ok {
            t!("storage_smart_ok").into_owned()
        } else {
            format!("{} · {}", t!("storage_smart_failing"), state)
        };
        ui::level_tag(if ok { Level::Healthy } else { Level::Warning }, text).small()
    });

    let has_io = d.read_bytes_per_sec > 0 || d.write_bytes_per_sec > 0;

    let header = h_flex()
        .gap_2()
        .flex_wrap()
        .items_center()
        .child(ui::chip(d.kind.clone()))
        .when_some(smart, |this, tag| this.child(tag));

    let usage = h_flex()
        .items_baseline()
        .gap_3()
        .flex_wrap()
        .child(
            div()
                .text_2xl()
                .font_semibold()
                .child(settings.bytes(d.used_bytes)),
        )
        .child(ui::muted(
            format!(
                "/ {} ({})",
                settings.bytes(d.total_bytes),
                fmt::percent(pct)
            ),
            cx,
        ))
        .child(
            h_flex()
                .ml_auto()
                .gap_4()
                .text_sm()
                .when(has_io, |this| {
                    this.child(ui::muted(
                        format!(
                            "{} {}",
                            t!("common_read"),
                            settings.rate(d.read_bytes_per_sec)
                        ),
                        cx,
                    ))
                    .child(ui::muted(
                        format!(
                            "{} {}",
                            t!("common_write"),
                            settings.rate(d.write_bytes_per_sec)
                        ),
                        cx,
                    ))
                })
                .when_some(d.temperature_c, |this, temp| {
                    this.child(ui::muted(settings.temp(temp), cx))
                }),
        );

    let items = vec![
        (
            t!("spec_mount").into_owned(),
            d.mount_point.clone().unwrap_or_default(),
        ),
        (
            t!("spec_filesystem").into_owned(),
            d.filesystem.clone().unwrap_or_default(),
        ),
        (t!("spec_kind").into_owned(), d.kind.clone()),
        (
            t!("spec_serial").into_owned(),
            d.serial.clone().unwrap_or_default(),
        ),
    ];

    ui::section(SharedString::from(format!("disk-{index}")), d.name.clone())
        .child(
            v_flex()
                .gap_3()
                .child(header)
                .child(usage)
                .child(ui::usage_bar(
                    SharedString::from(format!("disk-bar-{index}")),
                    pct,
                    cx,
                ))
                .child(ui::spec_list(items)),
        )
        .into_any_element()
}
