//! 概览：每个卷一张卡片，含容量占用、实时读写与 SMART 状态，卡片上可以直接去分析它。

use super::{StoragePage, volume_target};
use crate::health::Level;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, div, prelude::FluentBuilder as _,
};
use pc_specs_core::model::StorageInfo;
use rust_i18n::t;

/// 概览页的副标题和正文。
pub(super) fn body(
    page: &StoragePage,
    cx: &mut Context<StoragePage>,
) -> (Option<SharedString>, Vec<AnyElement>) {
    let entity = cx.entity();
    let s = page.app.read(cx);
    let disks = &s.snapshot.storages;
    if disks.is_empty() {
        return (
            None,
            vec![ui::empty_state(
                IconName::HardDrive,
                t!("storage_no_disks").into_owned(),
                "",
            )],
        );
    }
    (
        // 只有一个卷时「共 1 项」没有信息量。
        (disks.len() > 1)
            .then(|| SharedString::from(t!("count_found", n = disks.len()).into_owned())),
        disks
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let purgeable = d
                    .mount_point
                    .as_ref()
                    .and_then(|mount| page.purgeable.get(mount).copied());
                card(i, d, purgeable, s, &entity, cx)
            })
            .collect(),
    )
}

fn card(
    index: usize,
    d: &StorageInfo,
    purgeable: Option<u64>,
    s: &AppState,
    page: &Entity<StoragePage>,
    cx: &App,
) -> AnyElement {
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

    // 介质类型没识别出来、也没有 SMART 信息时，这一行就整个不要，免得留一道空白。
    let kind = fmt::storage_kind(&d.kind);
    let has_header = kind.is_some() || smart.is_some();
    let header = h_flex()
        .gap_2()
        .flex_wrap()
        .items_center()
        .when_some(kind, |this, kind| this.child(ui::chip(kind)))
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
                "/ {} ({}) · {} {}",
                settings.bytes(d.total_bytes),
                fmt::percent(pct),
                t!("spec_available"),
                settings.bytes(d.total_bytes.saturating_sub(d.used_bytes))
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

    let mut items = vec![
        (
            t!("spec_mount").into_owned(),
            d.mount_point.clone().unwrap_or_default(),
        ),
        (
            t!("spec_filesystem").into_owned(),
            d.filesystem.clone().unwrap_or_default(),
        ),
        (
            t!("spec_serial").into_owned(),
            d.serial.clone().unwrap_or_default(),
        ),
    ];
    // 可清除空间不是「没取到」的资料，没有就不列，免得被算进「几项未提供」。
    if let Some(bytes) = purgeable {
        items.push((t!("sa_purgeable_label").into_owned(), settings.bytes(bytes)));
    }

    // 有挂载点才能去分析、去测速。
    let actions = d
        .mount_point
        .as_deref()
        .filter(|mount| !mount.is_empty())
        .map(|mount| {
            let target = volume_target(&d.name, mount);
            let mount = mount.to_string();
            let (analyze_page, speed_page) = (page.clone(), page.clone());
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("disk-speed-{index}")))
                        .small()
                        .outline()
                        .icon(IconName::Gauge)
                        .label(t!("sa_speed_test_disk").into_owned())
                        .on_click(move |_, _, cx| {
                            let mount = mount.clone();
                            speed_page.update(cx, |page, cx| page.open_speed(mount, cx));
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("disk-analyze-{index}")))
                        .small()
                        .outline()
                        .icon(IconName::ChartPie)
                        .label(t!("sa_analyze_disk").into_owned())
                        .on_click(move |_, _, cx| {
                            let target = target.clone();
                            analyze_page.update(cx, |page, cx| page.start(target, cx));
                        }),
                )
        });

    ui::section(SharedString::from(format!("disk-{index}")), d.name.clone())
        .child(
            v_flex()
                .gap_3()
                .when(has_header, |this| this.child(header))
                .child(usage)
                .child(ui::usage_bar(
                    SharedString::from(format!("disk-bar-{index}")),
                    pct,
                    cx,
                ))
                .child(ui::spec_list(items, cx))
                .children(actions),
        )
        .into_any_element()
}
