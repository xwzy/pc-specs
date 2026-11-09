//! 导出：把完整快照导出为 Markdown / JSON，或渲染成 PNG 长图。

use crate::state::{AppState, ExportFormat};
use crate::ui;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div, prelude::FluentBuilder as _,
};
use rust_i18n::t;

pub fn render(s: &AppState, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let export = &s.export;
    let settings = &s.settings;
    let sensitive = settings.export_sensitive;

    // ---- 操作区 ----
    let format_state = state.clone();
    let format = ui::segmented(
        "export-format",
        vec![
            (ExportFormat::Markdown, t!("export_btn_md").into_owned()),
            (ExportFormat::Json, t!("export_btn_json").into_owned()),
        ],
        export.format,
        move |format, cx| {
            format_state.update(cx, |s, cx| s.generate_export(format, cx));
        },
    )
    .into_any_element();

    let action = |id: &'static str, icon: IconName, label: String| {
        Button::new(id).small().outline().icon(icon).label(label)
    };

    // 格式切换和「重新生成」只作用于预览，放在预览标题栏里，顶部操作区只留三个主操作。
    let regenerate = {
        let state = state.clone();
        let format = export.format;
        Button::new("export-regenerate")
            .small()
            .ghost()
            .icon(IconName::RefreshCw)
            .accessibility_label(t!("export_btn_refresh").into_owned())
            .tooltip(t!("export_btn_refresh").into_owned())
            .on_click(move |_, _, cx| state.update(cx, |s, cx| s.generate_export(format, cx)))
            .into_any_element()
    };
    let copy = {
        let state = state.clone();
        action(
            "export-copy",
            IconName::Copy,
            t!("export_btn_copy").into_owned(),
        )
        .on_click(move |_, _, cx| {
            state.update(cx, |s, cx| {
                if let Some(text) = s.export.text.clone() {
                    s.copy_to_clipboard(text.to_string(), cx);
                }
            })
        })
        .into_any_element()
    };
    let save = {
        let state = state.clone();
        action(
            "export-save",
            IconName::Download,
            t!("export_btn_save").into_owned(),
        )
        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.save_export(cx)))
        .into_any_element()
    };
    let png = {
        let state = state.clone();
        action(
            "export-png",
            IconName::Image,
            if export.poster_busy {
                t!("export_png_rendering").into_owned()
            } else {
                t!("export_btn_png").into_owned()
            },
        )
        .tooltip(t!("export_png_hint").into_owned())
        .loading(export.poster_busy)
        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.export_png(cx)))
        .into_any_element()
    };

    // ---- 敏感信息开关 ----
    let toggle_state = state.clone();
    let privacy = h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .child(Icon::new(IconName::ShieldAlert).text_color(if sensitive {
            cx.theme().warning
        } else {
            cx.theme().muted_foreground
        }))
        .child(div().flex_1().text_sm().child(if sensitive {
            format!(
                "{}{}",
                t!("export_sensitive_on"),
                t!("export_sensitive_keys")
            )
        } else {
            t!("export_sensitive_off").into_owned()
        }))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(ui::muted(t!("settings_export_sensitive").into_owned(), cx).text_xs())
                .child(Switch::new("export-sensitive").checked(sensitive).on_click(
                    move |checked, _, cx| {
                        let checked = *checked;
                        toggle_state.update(cx, |s, cx| {
                            s.update_settings(|v| v.export_sensitive = checked, cx);
                            // 开关变了，已经生成的预览就过期了，立刻按新设置重新生成。
                            s.generate_export(s.export.format, cx);
                        });
                    },
                )),
        );

    // ---- 预览 ----
    let chars = export.text.as_ref().map_or(0, |t| t.chars().count());
    let preview_title = h_flex()
        .items_center()
        .gap_2()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(t!("export_preview").into_owned()),
        )
        .child(ui::muted(format!("{chars} {}", t!("export_chars_suffix")), cx).text_xs())
        .when(sensitive, |this| {
            this.child(
                ui::level_tag(
                    crate::health::Level::Warning,
                    t!("export_tag_sensitive").into_owned(),
                )
                .small(),
            )
        })
        .child(
            h_flex()
                .ml_auto()
                .items_center()
                .gap_1()
                .child(format)
                .child(regenerate),
        );

    let preview_body = match &export.text {
        Some(text) => div()
            .id("export-preview")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p_4()
            .font_family(cx.theme().mono_font_family.clone())
            .text_xs()
            .child(text.clone())
            .into_any_element(),
        None => div()
            .flex_1()
            .p_4()
            .child(ui::muted(t!("export_generating").into_owned(), cx))
            .into_any_element(),
    };

    let preview = v_flex()
        .flex_1()
        .min_h_0()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().tokens.group_box)
        .child(preview_title)
        .child(preview_body);

    ui::page_fill(
        t!("export_title").into_owned(),
        Some(SharedString::from(t!("export_desc").into_owned())),
        vec![copy, save, png],
        v_flex().size_full().gap_3().child(privacy).child(preview),
        cx,
    )
}
