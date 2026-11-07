//! 设置：所有选项修改后立即生效并写入配置文件。

use crate::settings::{Language, NetUnit, Settings, SizeUnit, TempUnit, ThemePref};
use crate::state::{AppState, LookupState};
use crate::ui;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, rems,
};
use pc_specs_core::server::ServerStatus;
use rust_i18n::t;

pub fn render(state: &Entity<AppState>, cx: &App) -> AnyElement {
    let s = state.read(cx);
    let settings = &s.settings;

    // ---- 外观与语言 ----
    let appearance = ui::section(
        "settings-appearance",
        t!("settings_appearance").into_owned(),
    )
    .child(
        v_flex()
            .gap_4()
            .child(row(
                t!("settings_theme").into_owned(),
                None,
                choice(
                    state,
                    "settings-theme",
                    vec![
                        (ThemePref::System, t!("settings_theme_system").into_owned()),
                        (ThemePref::Light, t!("settings_theme_light").into_owned()),
                        (ThemePref::Dark, t!("settings_theme_dark").into_owned()),
                    ],
                    settings.theme,
                    |v, theme| v.theme = theme,
                ),
                cx,
            ))
            .child(row(
                t!("settings_language").into_owned(),
                Some(t!("settings_language_hint").into_owned()),
                choice(
                    state,
                    "settings-language",
                    vec![
                        (Language::Auto, t!("settings_theme_system").into_owned()),
                        (Language::En, "English".to_string()),
                        (Language::ZhCn, "简体中文".to_string()),
                    ],
                    settings.language,
                    |v, language| v.language = language,
                ),
                cx,
            ))
            .child(row(
                t!("settings_geek").into_owned(),
                Some(t!("settings_geek_hint").into_owned()),
                toggle(state, "settings-geek", settings.geek_mode, |v, on| {
                    v.geek_mode = on
                }),
                cx,
            )),
    );

    // ---- 单位 ----
    let units = ui::section("settings-units", t!("settings_units").into_owned()).child(
        v_flex()
            .gap_4()
            .child(row(
                t!("settings_units_temp").into_owned(),
                None,
                choice(
                    state,
                    "settings-temp",
                    vec![
                        (TempUnit::Celsius, t!("settings_unit_c").into_owned()),
                        (TempUnit::Fahrenheit, t!("settings_unit_f").into_owned()),
                    ],
                    settings.temp_unit,
                    |v, unit| v.temp_unit = unit,
                ),
                cx,
            ))
            .child(row(
                t!("settings_units_bytes").into_owned(),
                None,
                choice(
                    state,
                    "settings-size",
                    vec![
                        (SizeUnit::Binary, t!("settings_unit_gib").into_owned()),
                        (SizeUnit::Decimal, t!("settings_unit_gb").into_owned()),
                    ],
                    settings.size_unit,
                    |v, unit| v.size_unit = unit,
                ),
                cx,
            ))
            .child(row(
                t!("settings_units_netspeed").into_owned(),
                None,
                choice(
                    state,
                    "settings-net",
                    vec![
                        (
                            NetUnit::BytesPerSec,
                            t!("settings_unit_bytes_s").into_owned(),
                        ),
                        (NetUnit::BitsPerSec, t!("settings_unit_bits_s").into_owned()),
                    ],
                    settings.net_unit,
                    |v, unit| v.net_unit = unit,
                ),
                cx,
            )),
    );

    // ---- 实时监控 ----
    let monitor = ui::section("settings-monitor", t!("settings_monitor").into_owned()).child(
        v_flex()
            .gap_4()
            .child(row(
                t!("settings_interval").into_owned(),
                Some(t!("settings_interval_hint").into_owned()),
                choice(
                    state,
                    "settings-interval",
                    [500u64, 1000, 2000, 5000]
                        .into_iter()
                        .map(|ms| (ms, format!("{} s", ms as f64 / 1000.0)))
                        .collect(),
                    settings.interval_ms,
                    |v, ms| v.interval_ms = ms,
                ),
                cx,
            ))
            .child(row(
                t!("settings_window").into_owned(),
                Some(t!("settings_window_hint").into_owned()),
                choice(
                    state,
                    "settings-window",
                    [1u32, 5, 10]
                        .into_iter()
                        .map(|m| (m, t!("unit_minutes", n = m).into_owned()))
                        .collect(),
                    settings.history_minutes,
                    |v, minutes| v.history_minutes = minutes,
                ),
                cx,
            ))
            .child(row(
                t!("settings_alerts").into_owned(),
                Some(t!("settings_alerts_hint").into_owned()),
                toggle(state, "settings-alerts", settings.alerts, |v, on| {
                    v.alerts = on
                }),
                cx,
            )),
    );

    // ---- 托盘与桌面小组件 ----
    let autostart_state = state.clone();
    let mut tray = v_flex()
        .gap_4()
        .child(row(
            t!("settings_autostart").into_owned(),
            Some(t!("settings_autostart_hint").into_owned()),
            Switch::new("settings-autostart")
                .checked(s.autostart)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    autostart_state.update(cx, |s, cx| s.set_autostart(checked, cx));
                })
                .into_any_element(),
            cx,
        ))
        .child(row(
            t!("settings_tray_enabled").into_owned(),
            Some(t!("settings_tray_enabled_hint").into_owned()),
            toggle(state, "settings-tray", settings.tray_enabled, |v, on| {
                v.tray_enabled = on
            }),
            cx,
        ));
    if settings.tray_enabled {
        tray = tray
            .child(row(
                t!("settings_close_to_tray").into_owned(),
                Some(t!("settings_close_to_tray_hint").into_owned()),
                toggle(
                    state,
                    "settings-close-to-tray",
                    settings.close_to_tray,
                    |v, on| v.close_to_tray = on,
                ),
                cx,
            ))
            .child({
                let state = state.clone();
                Button::new("settings-configure-bar")
                    .small()
                    .outline()
                    .label(t!("bar_configure").into_owned())
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| s.set_page(crate::state::Page::MenuBar, cx))
                    })
                    .into_any_element()
            });
    }
    let tray = ui::section("settings-tray", t!("settings_tray_section").into_owned()).child(tray);

    let floating = ui::section(
        "settings-floating",
        t!("settings_floating_section").into_owned(),
    )
    .child(row(
        t!("settings_floating_net").into_owned(),
        Some(t!("settings_floating_net_hint").into_owned()),
        toggle(
            state,
            "settings-floating",
            settings.floating_net,
            |v, on| v.floating_net = on,
        ),
        cx,
    ));

    // ---- 本地 HTTP 接口与公网 IP ----
    let status = match s.server_status() {
        None => t!("settings_server_off").into_owned(),
        Some(ServerStatus::Starting) => t!("settings_server_starting").into_owned(),
        Some(ServerStatus::Listening(addr)) => {
            t!("settings_server_listening", addr = addr.to_string()).into_owned()
        }
        Some(ServerStatus::Failed(error)) => {
            t!("settings_server_failed", error = error).into_owned()
        }
    };
    let ip_state = state.clone();
    let clear_state = state.clone();
    let fetching = s.public_ip == LookupState::Loading;
    let known_ip = match &s.public_ip {
        LookupState::Done(ip) => ip.clone(),
        _ => s.snapshot.network.public_ip.clone(),
    };
    let network = ui::section("settings-network", t!("settings_page_network").into_owned()).child(
        v_flex()
            .gap_4()
            .child(row(
                t!("settings_server").into_owned(),
                Some(t!("settings_server_hint").into_owned()),
                toggle(
                    state,
                    "settings-server",
                    settings.server_enabled,
                    |v, on| v.server_enabled = on,
                ),
                cx,
            ))
            .when(settings.server_enabled, |this| {
                this.child(row(
                    t!("settings_server_lan").into_owned(),
                    Some(t!("settings_server_lan_hint").into_owned()),
                    toggle(
                        state,
                        "settings-server-lan",
                        settings.server_lan,
                        |v, on| v.server_lan = on,
                    ),
                    cx,
                ))
                .child(row(
                    t!("settings_server_status").into_owned(),
                    None,
                    ui::muted(status.clone(), cx).into_any_element(),
                    cx,
                ))
            })
            .child(row(
                t!("net_query_public").into_owned(),
                Some(t!("settings_public_ip_hint").into_owned()),
                h_flex()
                    .gap_2()
                    .items_center()
                    .when_some(known_ip, |this, ip| this.child(div().text_sm().child(ip)))
                    .child(
                        Button::new("settings-ip-fetch")
                            .small()
                            .outline()
                            .label(t!("settings_public_ip_fetch").into_owned())
                            .loading(fetching)
                            .on_click(move |_, _, cx| {
                                ip_state.update(cx, |s, cx| s.fetch_public_ip(cx));
                            }),
                    )
                    .child(
                        Button::new("settings-ip-clear")
                            .small()
                            .ghost()
                            .label(t!("settings_public_ip_clear").into_owned())
                            .on_click(move |_, _, cx| {
                                clear_state.update(cx, |s, cx| s.clear_public_ip(cx));
                            }),
                    )
                    .into_any_element(),
                cx,
            )),
    );

    // ---- 导出 ----
    let export = ui::section(
        "settings-export",
        t!("settings_export_section").into_owned(),
    )
    .child(row(
        t!("settings_export_sensitive").into_owned(),
        Some(t!("settings_export_sensitive_hint").into_owned()),
        toggle(
            state,
            "settings-export-sensitive",
            settings.export_sensitive,
            |v, on| v.export_sensitive = on,
        ),
        cx,
    ));

    // ---- 关于 ----
    let config_path = Settings::path();
    let reveal = config_path.clone().map(|path| {
        Button::new("settings-reveal")
            .small()
            .outline()
            .label(t!("settings_about_reveal").into_owned())
            .on_click(move |_, _, cx| cx.reveal_path(&path))
            .into_any_element()
    });
    let about = ui::section("settings-about", t!("settings_about").into_owned()).child(
        v_flex()
            .gap_3()
            .child(ui::muted(t!("settings_about_desc").into_owned(), cx))
            .child(ui::spec_list(vec![
                (
                    t!("settings_about_version").into_owned(),
                    env!("CARGO_PKG_VERSION").to_string(),
                ),
                (
                    t!("settings_about_platform").into_owned(),
                    format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH),
                ),
                (
                    t!("settings_about_config").into_owned(),
                    config_path
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ]))
            .when_some(reveal, |this, button| this.child(h_flex().child(button)))
            .child(ui::muted(t!("settings_about_privacy").into_owned(), cx).text_xs()),
    );

    ui::page(
        "page-settings",
        t!("settings_title").into_owned(),
        None,
        vec![],
        vec![
            appearance.into_any_element(),
            units.into_any_element(),
            monitor.into_any_element(),
            tray.into_any_element(),
            floating.into_any_element(),
            network.into_any_element(),
            export.into_any_element(),
            about.into_any_element(),
        ],
        cx,
    )
}

/// 左侧标题 / 说明，右侧控件。
fn row(title: String, description: Option<String>, control: AnyElement, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_6()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().font_medium().child(title))
                .when_some(description, |this, text| {
                    this.child(
                        div()
                            .max_w(rems(36.))
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(text),
                    )
                }),
        )
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}

fn toggle(
    state: &Entity<AppState>,
    id: &'static str,
    value: bool,
    set: fn(&mut Settings, bool),
) -> AnyElement {
    let state = state.clone();
    Switch::new(id)
        .checked(value)
        .on_click(move |checked, _, cx| {
            let checked = *checked;
            state.update(cx, |s, cx| s.update_settings(|v| set(v, checked), cx));
        })
        .into_any_element()
}

fn choice<T: Copy + PartialEq + 'static>(
    state: &Entity<AppState>,
    id: &'static str,
    options: Vec<(T, String)>,
    current: T,
    set: fn(&mut Settings, T),
) -> AnyElement {
    let state = state.clone();
    ui::segmented(id, options, current, move |value, cx| {
        state.update(cx, |s, cx| s.update_settings(|v| set(v, value), cx));
    })
    .into_any_element()
}
