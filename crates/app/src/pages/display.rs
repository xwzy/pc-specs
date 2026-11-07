//! 显示器：布局预览与每块屏幕的分辨率、刷新率、缩放、物理尺寸。

use crate::state::AppState;
use crate::ui;
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, prelude::FluentBuilder as _, rems,
};
use pc_specs_core::model::DisplayInfo;
use rust_i18n::t;

/// 预览框最大尺寸（rem）。
const PREVIEW_W: f32 = 14.0;
const PREVIEW_H: f32 = 9.0;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let displays = &s.snapshot.displays;
    if displays.is_empty() {
        return ui::page(
            "page-display",
            t!("nav_display").into_owned(),
            None,
            vec![],
            vec![ui::empty_state(
                IconName::Monitor,
                t!("disp_no_data").into_owned(),
                t!("disp_no_data_hint").into_owned(),
            )],
            cx,
        );
    }

    let max_w = displays
        .iter()
        .map(|d| d.width_px)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let max_h = displays
        .iter()
        .map(|d| d.height_px)
        .max()
        .unwrap_or(1)
        .max(1) as f32;

    let preview =
        ui::section("disp-preview", t!("disp_layout_preview").into_owned()).child(
            h_flex().flex_wrap().items_end().gap_3().py_2().children(
                displays.iter().enumerate().map(|(i, d)| {
                    let w = (d.width_px as f32 / max_w * PREVIEW_W).max(4.0);
                    let h = (d.height_px as f32 / max_h * PREVIEW_H).max(2.5);
                    let border = if d.is_primary {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    };
                    v_flex()
                        .id(SharedString::from(format!("disp-box-{i}")))
                        .w(rems(w))
                        .h(rems(h))
                        .items_center()
                        .justify_center()
                        .gap_0p5()
                        .rounded(cx.theme().radius)
                        .border_2()
                        .border_color(border)
                        .text_xs()
                        .when(d.is_primary, |this| this.text_color(cx.theme().primary))
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .when(d.is_primary, |this| {
                                    this.child(Icon::new(IconName::Star).xsmall())
                                })
                                .child(format!("{}×{}", d.width_px, d.height_px)),
                        )
                        .when_some(
                            d.scale_factor.filter(|f| (*f - 1.0).abs() > 0.01),
                            |this, f| this.child(ui::dim(format!("{f:.1}×"), cx)),
                        )
                        .into_any_element()
                }),
            ),
        );

    let mut body = vec![preview.into_any_element()];
    body.extend(displays.iter().enumerate().map(|(i, d)| card(i, d)));

    ui::page(
        "page-display",
        t!("nav_display").into_owned(),
        Some(SharedString::from(
            t!("count_found", n = displays.len()).into_owned(),
        )),
        vec![],
        body,
        cx,
    )
}

fn card(index: usize, d: &DisplayInfo) -> AnyElement {
    let mut items = vec![
        (
            t!("spec_resolution").into_owned(),
            format!("{} × {}", d.width_px, d.height_px),
        ),
        (
            t!("spec_refresh").into_owned(),
            d.refresh_hz.map_or(String::new(), |hz| format!("{hz} Hz")),
        ),
        (
            t!("disp_scale").into_owned(),
            d.scale_factor.map_or(String::new(), |f| format!("{f:.2}×")),
        ),
        (
            t!("disp_logical").into_owned(),
            d.scale_factor
                .filter(|f| *f > 0.0)
                .map_or(String::new(), |f| {
                    format!(
                        "{} × {}",
                        (d.width_px as f32 / f).round() as u32,
                        (d.height_px as f32 / f).round() as u32
                    )
                }),
        ),
        (
            t!("disp_color").into_owned(),
            d.color_depth.map_or(String::new(), |c| format!("{c}-bit")),
        ),
    ];
    if let (Some(w_mm), Some(h_mm)) = (d.physical_width_mm, d.physical_height_mm)
        && w_mm > 0
        && h_mm > 0
    {
        items.push((
            t!("disp_physical").into_owned(),
            format!("{w_mm} × {h_mm} mm"),
        ));
        let diagonal_in = ((w_mm as f32).powi(2) + (h_mm as f32).powi(2)).sqrt() / 25.4;
        items.push((
            t!("disp_diagonal").into_owned(),
            format!("{diagonal_in:.1}″"),
        ));
        let ppi = d.width_px as f32 / (w_mm as f32 / 25.4);
        items.push((t!("disp_ppi").into_owned(), format!("{ppi:.0} PPI")));
    }

    ui::section(SharedString::from(format!("disp-{index}")), d.name.clone())
        .child(
            v_flex()
                .gap_3()
                .when(d.is_primary, |this| {
                    this.child(h_flex().child(ui::chip(t!("disp_primary").into_owned())))
                })
                .child(ui::spec_list(items)),
        )
        .into_any_element()
}
