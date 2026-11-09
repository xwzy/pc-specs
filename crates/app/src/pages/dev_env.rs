//! 开发环境：已安装的语言运行时、包管理器、版本控制、编辑器、容器与 Shell。

use crate::state::AppState;
use crate::ui;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
    prelude::FluentBuilder as _, rems,
};
use pc_specs_core::model::{DevEnvInfo, RuntimeInfo};
use rust_i18n::t;

pub fn render(s: &AppState, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let env = &s.snapshot.dev_env;
    let groups: [(String, &Vec<RuntimeInfo>); 6] = [
        (t!("dev_languages").into_owned(), &env.languages),
        (
            t!("dev_package_managers").into_owned(),
            &env.package_managers,
        ),
        (t!("dev_vcs").into_owned(), &env.vcs),
        (t!("dev_editors").into_owned(), &env.editors),
        (t!("dev_containers").into_owned(), &env.containers),
        (t!("dev_shells").into_owned(), &env.shells),
    ];

    let summary = summary_text(env);
    let copy_state = state.clone();
    let copy = Button::new("dev-copy")
        .small()
        .outline()
        .icon(IconName::Copy)
        .label(t!("dev_copy_summary").into_owned())
        .on_click(move |_, _, cx| {
            let text = summary.clone();
            copy_state.update(cx, |s, cx| s.copy_to_clipboard(text, cx));
        })
        .into_any_element();

    let cards = groups.iter().enumerate().map(|(i, (title, list))| {
        let content = if list.is_empty() {
            ui::muted(t!("dev_no_data").into_owned(), cx).into_any_element()
        } else {
            v_flex()
                .gap_2()
                .children(
                    list.iter()
                        .map(|r| runtime_row(r, s.settings.geek_mode, cx)),
                )
                .into_any_element()
        };
        ui::section(SharedString::from(format!("dev-group-{i}")), title.clone())
            .flex_1()
            .min_w(rems(22.))
            .child(content)
            .into_any_element()
    });

    let mut body = vec![
        h_flex()
            .flex_wrap()
            .items_start()
            .gap_4()
            .children(cards)
            .into_any_element(),
    ];

    if !env.env_keys.is_empty() {
        body.push(
            ui::section(
                "dev-env-keys",
                format!("{} ({})", t!("dev_env_keys"), env.env_keys.len()),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex().flex_wrap().gap_1p5().children(
                            env.env_keys
                                .iter()
                                .map(|k| ui::chip(k.clone()).into_any_element()),
                        ),
                    )
                    .child(ui::muted(t!("dev_env_keys_hint").into_owned(), cx).text_xs()),
            )
            .into_any_element(),
        );
    }

    ui::page(
        "page-dev-env",
        t!("nav_dev_env").into_owned(),
        None,
        vec![copy],
        body,
        cx,
    )
}

fn runtime_row(r: &RuntimeInfo, geek: bool, cx: &App) -> AnyElement {
    let short = r.version.as_deref().map(short_version);
    // 极客模式下把完整的原始输出和路径一并列出来。
    let raw = r
        .version
        .clone()
        .filter(|raw| short.as_deref() != Some(raw.trim()));
    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().flex_shrink_0().font_semibold().child(r.name.clone()))
                .child(match short {
                    Some(v) => ui::chip(v).into_any_element(),
                    None => ui::muted("—", cx).into_any_element(),
                }),
        )
        .when(geek, |this| {
            this.when_some(raw, |this, raw| {
                this.child(ui::muted(raw, cx).text_xs().truncate())
            })
            .when_some(r.path.clone(), |this, path| {
                this.child(ui::muted(path, cx).text_xs().truncate())
            })
        })
        .into_any_element()
}

/// `--version` 的原始输出常带构建信息与路径，列表里只留第一个版本号。
fn short_version(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut dots = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                i += 1;
            } else if bytes[i] == b'.' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit()) {
                dots += 1;
                i += 1;
            } else {
                break;
            }
        }
        if dots > 0 {
            return raw[start..i].to_string();
        }
    }
    let trimmed = raw.trim();
    if trimmed.chars().count() > 24 {
        format!("{}…", trimmed.chars().take(24).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

/// 可复制的纯文本摘要（Markdown 列表）。
fn summary_text(env: &DevEnvInfo) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut block = |title: &str, list: &[RuntimeInfo]| {
        if list.is_empty() {
            return;
        }
        lines.push(format!("## {title}"));
        for r in list {
            lines.push(format!(
                "- {}: {}",
                r.name,
                r.version
                    .as_deref()
                    .map_or_else(|| "unknown".to_string(), short_version)
            ));
        }
        lines.push(String::new());
    };
    block("Languages", &env.languages);
    block("Package Managers", &env.package_managers);
    block("VCS", &env.vcs);
    block("Editors", &env.editors);
    block("Containers", &env.containers);
    block("Shells", &env.shells);
    if !env.env_keys.is_empty() {
        lines.push("## Env Keys".to_string());
        lines.push(env.env_keys.join(", "));
    }
    lines.join("\n")
}
