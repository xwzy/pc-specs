//! 操作系统与主机信息。

use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::component::{StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div, rems,
};
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let os = &s.snapshot.os;
    let host = &s.snapshot.host;

    let stat = |label: String, value: String, sub: String| {
        v_flex()
            .flex_1()
            .min_w(rems(12.))
            .gap_1()
            .p_4()
            .child(ui::muted(label, cx).text_xs())
            .child(div().text_lg().font_semibold().truncate().child(value))
            .child(ui::muted(sub, cx).text_xs().truncate())
    };

    let uptime = fmt::duration(host.uptime_secs);
    let boot = fmt::datetime(host.boot_time);

    let overview = ui::section("os-overview", t!("nav_dashboard").into_owned()).child(
        h_flex()
            .flex_wrap()
            .items_stretch()
            .child(stat(
                t!("os_family").into_owned(),
                family_label(&os.family),
                os.name.clone(),
            ))
            .child(stat(
                t!("spec_uptime").into_owned(),
                uptime.clone(),
                format!("{} {}", t!("os_since"), boot),
            ))
            .child(stat(
                t!("spec_hostname").into_owned(),
                host.hostname.clone(),
                host.username.clone(),
            ))
            .child(stat(
                t!("spec_locale").into_owned(),
                os.locale.clone(),
                os.desktop.clone().unwrap_or_default(),
            )),
    );

    let details =
        ui::plain_section("os-details", t!("cpu_details").into_owned()).child(ui::spec_list(vec![
            (t!("os_family").into_owned(), family_label(&os.family)),
            (t!("os_name").into_owned(), os.name.clone()),
            (t!("os_version").into_owned(), os.version.clone()),
            (t!("spec_kernel").into_owned(), os.kernel.clone()),
            (t!("spec_arch").into_owned(), os.arch.clone()),
            (t!("spec_locale").into_owned(), os.locale.clone()),
            (
                t!("spec_shell").into_owned(),
                os.shell.clone().unwrap_or_default(),
            ),
            (
                t!("spec_desktop").into_owned(),
                os.desktop.clone().unwrap_or_default(),
            ),
            (t!("spec_hostname").into_owned(), host.hostname.clone()),
            (t!("spec_user").into_owned(), host.username.clone()),
            (t!("spec_uptime").into_owned(), uptime),
            (t!("spec_boot_time").into_owned(), boot),
            (t!("os_app_version").into_owned(), host.app_version.clone()),
        ]));

    ui::page(
        "page-os",
        t!("nav_os").into_owned(),
        Some(SharedString::from(format!("{} · {}", os.name, os.arch))),
        vec![],
        vec![overview.into_any_element(), details.into_any_element()],
        cx,
    )
}

/// `macos` → `macOS` 这类展示用名称。
fn family_label(family: &str) -> String {
    match family {
        "macos" => "macOS".to_string(),
        "windows" => "Windows".to_string(),
        "linux" => "Linux".to_string(),
        other => other.to_string(),
    }
}
