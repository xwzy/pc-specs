use super::{
    color,
    config::{ItemStyle, MenuBarItem, MenuBarSettings, Metric, SPACING_RANGE, SPACING_STEP},
    data::{self, Scale, Value},
    icon,
    widgets::{self, Palette},
};
use crate::{state::AppState, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Div, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _,
    div, prelude::FluentBuilder as _, px, relative,
};
use rust_i18n::t;

pub fn render(state: &Entity<AppState>, cx: &App) -> AnyElement {
    let s = state.read(cx);
    let config = &s.settings.menu_bar;
    let preview = ui::section("bar-preview", t!("bar_preview").into_owned()).child(
        v_flex()
            .gap_3()
            .child(preview(s, cx))
            .child(ui::muted(t!("bar_preview_hint").into_owned(), cx).text_xs()),
    );
    let general = ui::section("bar-general", t!("bar_general").into_owned()).child(
        v_flex()
            .gap_4()
            .child({
                let state = state.clone();
                let row = h_flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(div().text_sm().child(t!("bar_system").into_owned()))
                    .child(
                        Switch::new("bar-enabled")
                            .accessibility_label(t!("bar_system").into_owned())
                            .checked(config.enabled && s.settings.tray_enabled)
                            .on_click(move |checked, _, cx| {
                                let checked = *checked;
                                state.update(cx, |s, cx| {
                                    s.update_settings(
                                        |settings| {
                                            settings.menu_bar.enabled = checked;
                                            if checked {
                                                settings.tray_enabled = true;
                                            }
                                        },
                                        cx,
                                    )
                                });
                            }),
                    );
                let hint = if cfg!(target_os = "macos") {
                    t!("bar_system_mac")
                } else {
                    t!("bar_system_other")
                };
                hinted(row.into_any_element(), hint.into_owned(), cx)
            })
            .when(cfg!(target_os = "macos"), |this| {
                this.child(hinted(
                    toggle(
                        state,
                        "bar-icon",
                        t!("bar_show_icon").into_owned(),
                        config.show_icon,
                        |c, v| c.show_icon = v,
                        cx,
                    ),
                    t!("bar_show_icon_hint").into_owned(),
                    cx,
                ))
            })
            .child(toggle(
                state,
                "bar-labels",
                t!("bar_labels").into_owned(),
                config.show_labels,
                |c, v| c.show_labels = v,
                cx,
            ))
            .child(toggle(
                state,
                "bar-units",
                t!("bar_units").into_owned(),
                config.show_units,
                |c, v| c.show_units = v,
                cx,
            ))
            .child(toggle(
                state,
                "bar-separators",
                t!("bar_separators").into_owned(),
                config.separators,
                |c, v| c.separators = v,
                cx,
            ))
            .child(toggle(
                state,
                "bar-colorful",
                t!("bar_colorful").into_owned(),
                config.colorful,
                |c, v| c.colorful = v,
                cx,
            ))
            .when(cfg!(target_os = "macos"), |this| {
                this.child(hinted(
                    stepper(
                        state,
                        "bar-spacing",
                        t!("bar_spacing").into_owned(),
                        config.spacing,
                        |c, v| c.spacing = v,
                        cx,
                    ),
                    t!("bar_spacing_hint").into_owned(),
                    cx,
                ))
            })
            .child(choice(
                state,
                "bar-refresh",
                t!("bar_refresh").into_owned(),
                [1000u64, 2000, 5000]
                    .into_iter()
                    .map(|n| (n, format!("{} s", n / 1000)))
                    .collect(),
                config.refresh_ms,
                |c, v| c.refresh_ms = v,
            ))
            .child(choice(
                state,
                "bar-process-count",
                t!("bar_process_count").into_owned(),
                [5usize, 8, 12]
                    .into_iter()
                    .map(|n| (n, n.to_string()))
                    .collect(),
                config.process_rows,
                |c, v| c.process_rows = v,
            )),
    );
    let count = config.items.len();
    let items = v_flex().children(
        config
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| item_card(s, state, index, count, item, cx)),
    );
    let reset = state.clone();
    ui::page(
        "page-menu-bar",
        t!("bar_title").into_owned(),
        Some(t!("bar_description").into_owned().into()),
        vec![
            Button::new("bar-reset")
                .small()
                .outline()
                .label(t!("bar_reset").into_owned())
                .on_click(move |_, _, cx| {
                    reset.update(cx, |s, cx| {
                        s.update_settings(|v| v.menu_bar = MenuBarSettings::default(), cx)
                    })
                })
                .into_any_element(),
        ],
        vec![
            preview.into_any_element(),
            general.into_any_element(),
            ui::section("bar-items", t!("bar_items").into_owned())
                .child(items)
                .into_any_element(),
        ],
        cx,
    )
}

/// 菜单栏位图里实际画出来的文字。
fn bar_text(value: &Value, show_units: bool) -> String {
    format!("{}{}{}", value.lead, value.num, value.bar_unit(show_units))
}

/// 最近一小段走势，按和菜单栏位图同样的量程缩放到 0..=1。
fn trend(s: &AppState, item: &MenuBarItem, scale: Scale) -> Vec<Option<f32>> {
    let skip = s.ticks.len().saturating_sub(12);
    let raw: Vec<Option<f64>> = s
        .ticks
        .iter()
        .skip(skip)
        .map(|tick| {
            data::reading(item.metric, item.show, Some(tick), &s.snapshot, &s.settings).number
        })
        .collect();
    let window: Vec<f64> = raw.iter().flatten().copied().collect();
    raw.into_iter()
        .map(|v| v.map(|v| scale.fraction(v, &window)))
        .collect()
}

/// 一个菜单栏项的预览，版式和菜单栏上画出来的一致。
fn preview_item(s: &AppState, item: &MenuBarItem, cx: &App) -> Div {
    let config = &s.settings.menu_bar;
    let theme = cx.theme();
    let metric = item.metric;
    let reading = data::reading(
        metric,
        item.show,
        s.latest.as_ref(),
        &s.snapshot,
        &s.settings,
    );
    let ink = if config.colorful {
        color(metric, cx)
    } else {
        theme.foreground
    };
    let mut lines = vec![bar_text(&reading.value, config.show_units)];
    lines.extend(
        reading
            .second
            .as_ref()
            .map(|value| bar_text(value, config.show_units)),
    );
    let number = |text: String, size: f32| {
        widgets::tabular(
            div()
                .text_size(px(size))
                .line_height(relative(1.))
                .font_semibold()
                .text_color(ink)
                .child(text),
        )
    };
    let tag = |size: f32| {
        div()
            .text_size(px(size))
            .line_height(relative(1.))
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(metric.label())
    };
    match item.style {
        ItemStyle::Stacked => {
            let column = v_flex().items_center().justify_center().gap(px(3.));
            if lines.len() > 1 {
                column.children(lines.into_iter().map(|line| number(line, 11.)))
            } else {
                column
                    .child(number(lines.swap_remove(0), 13.))
                    .when(config.show_labels, |column| column.child(tag(8.5)))
            }
        }
        ItemStyle::Inline => h_flex()
            .items_center()
            .gap(px(6.))
            .when(config.show_labels, |row| row.child(tag(10.)))
            .children(lines.into_iter().map(|line| number(line, 13.))),
        ItemStyle::Graph => h_flex()
            .items_center()
            .gap(px(6.))
            .child(div().w(px(26.)).h(px(16.)).child(widgets::columns(
                trend(s, item, reading.scale),
                12,
                4,
                (1., 1.),
                Palette::new(ink, cx).plain(),
            )))
            .child(
                v_flex()
                    .items_end()
                    .gap(px(3.))
                    .children(lines.into_iter().map(|line| number(line, 12.))),
            ),
    }
}

/// 把菜单栏上的样子原样摆出来，设置一改，预览马上跟着变。
fn preview(s: &AppState, cx: &App) -> Div {
    let config = &s.settings.menu_bar;
    let theme = cx.theme();
    let pad = px(config.padding());
    let bar = h_flex()
        .h(px(46.))
        .px(px(8.))
        .items_center()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .when(!config.enabled, |bar| bar.opacity(0.5));
    let mut shown = config.visible().peekable();
    if shown.peek().is_none() {
        return bar.child(ui::muted(t!("bar_no_items").into_owned(), cx).text_xs());
    }
    let mut bar = bar;
    if cfg!(target_os = "macos") && config.icon_visible() {
        bar = bar.child(
            Icon::new(IconName::Cpu)
                .size(px(16.))
                .text_color(theme.foreground)
                .mx(pad),
        );
    }
    // 和菜单栏上一样：每一项左右各留一半间距，分隔线正好落在两项中间。
    for (index, item) in shown.enumerate() {
        if index > 0 && config.separators {
            bar = bar.child(div().w(px(1.)).h(px(18.)).bg(theme.border));
        }
        bar = bar.child(preview_item(s, item, cx).px(pad));
    }
    bar
}

fn edit(
    state: &Entity<AppState>,
    metric: Metric,
    change: impl FnOnce(&mut MenuBarItem),
    cx: &mut App,
) {
    state.update(cx, |s, cx| {
        s.update_settings(
            |settings| {
                if let Some(item) = settings
                    .menu_bar
                    .items
                    .iter_mut()
                    .find(|i| i.metric == metric)
                {
                    change(item);
                }
            },
            cx,
        )
    });
}

fn item_card(
    s: &AppState,
    state: &Entity<AppState>,
    index: usize,
    count: usize,
    item: &MenuBarItem,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let metric = item.metric;
    let key = metric.key();
    let accent = color(metric, cx);
    let current = data::reading(
        metric,
        item.show,
        s.latest.as_ref(),
        &s.snapshot,
        &s.settings,
    )
    .plain();
    let mut order = h_flex().gap_1().w(px(64.)).justify_end();
    if index > 0 {
        let state = state.clone();
        order = order.child(
            Button::new(SharedString::from(format!("up-{key}")))
                .ghost()
                .small()
                .icon(IconName::ArrowUp)
                .on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| {
                        s.update_settings(|v| v.menu_bar.move_item(metric, true), cx)
                    })
                }),
        );
    }
    if index + 1 < count {
        let state = state.clone();
        order = order.child(
            Button::new(SharedString::from(format!("down-{key}")))
                .ghost()
                .small()
                .icon(IconName::ArrowDown)
                .on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| {
                        s.update_settings(|v| v.menu_bar.move_item(metric, false), cx)
                    })
                }),
        );
    }
    let option = |name: String, control: AnyElement| {
        h_flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(64.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(name),
            )
            .child(control)
    };
    let styles = {
        let state = state.clone();
        ui::segmented(
            SharedString::from(format!("style-{key}")),
            vec![
                (ItemStyle::Stacked, t!("bar_stacked").into_owned()),
                (ItemStyle::Inline, t!("bar_inline").into_owned()),
                (ItemStyle::Graph, t!("bar_graph").into_owned()),
            ],
            item.style,
            move |style, cx| edit(&state, metric, |i| i.style = style, cx),
        )
        .into_any_element()
    };
    let contents = (metric.shows().len() > 1).then(|| {
        let state = state.clone();
        ui::segmented(
            SharedString::from(format!("show-{key}")),
            metric
                .shows()
                .iter()
                .map(|&show| (show, show.title(metric)))
                .collect(),
            item.show,
            move |show, cx| edit(&state, metric, |i| i.show = show, cx),
        )
        .into_any_element()
    });
    let toggle_state = state.clone();
    v_flex()
        .w_full()
        .gap_3()
        .py_3()
        .when(index + 1 < count, |card| {
            card.border_b_1().border_color(theme.border)
        })
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .child(
                    Switch::new(SharedString::from(format!("enabled-{key}")))
                        .accessibility_label(metric.title())
                        .checked(item.enabled)
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            edit(&toggle_state, metric, |i| i.enabled = checked, cx);
                        }),
                )
                .child(
                    div()
                        .size(px(30.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .bg(accent.opacity(0.16))
                        .child(Icon::new(icon(metric)).small().text_color(accent)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().font_medium().child(metric.title()))
                        .child(ui::muted(current, cx).text_xs().truncate()),
                )
                .child(order),
        )
        .when(item.enabled, |card| {
            card.child(
                v_flex()
                    .gap_2()
                    .pl(px(42.))
                    .child(option(t!("bar_style").into_owned(), styles))
                    .children(
                        contents.map(|control| option(t!("bar_content").into_owned(), control)),
                    ),
            )
        })
}

/// 一行设置加上紧贴在它下面的说明：说明跟这一行靠得近，跟下一行隔开。
fn hinted(row: AnyElement, hint: String, cx: &App) -> Div {
    v_flex()
        .gap_1()
        .child(row)
        .child(ui::muted(hint, cx).text_xs())
}

fn toggle(
    state: &Entity<AppState>,
    id: &'static str,
    label: String,
    value: bool,
    set: fn(&mut MenuBarSettings, bool),
    cx: &App,
) -> AnyElement {
    let state = state.clone();
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(div().text_sm().child(label.clone()))
        .child(
            Switch::new(id)
                .accessibility_label(label)
                .checked(value)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    state.update(cx, |s, cx| {
                        s.update_settings(|v| set(&mut v.menu_bar, checked), cx)
                    });
                }),
        )
        .text_color(cx.theme().foreground)
        .into_any_element()
}

/// 「− 10 pt +」：每按一下改一个步长，到头了按钮变灰。
fn stepper(
    state: &Entity<AppState>,
    id: &'static str,
    label: String,
    value: u32,
    set: fn(&mut MenuBarSettings, u32),
    cx: &App,
) -> AnyElement {
    let button = |name: &str, icon: IconName, enabled: bool, next: u32| {
        let state = state.clone();
        Button::new(SharedString::from(format!("{id}-{name}")))
            .small()
            .outline()
            .icon(icon)
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                state.update(cx, |s, cx| {
                    s.update_settings(|v| set(&mut v.menu_bar, next), cx)
                })
            })
    };
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .text_color(cx.theme().foreground)
        .child(div().text_sm().child(label))
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(button(
                    "down",
                    IconName::Minus,
                    value > *SPACING_RANGE.start(),
                    value.saturating_sub(SPACING_STEP),
                ))
                .child(widgets::tabular(
                    div()
                        .w(px(56.))
                        .text_center()
                        .text_sm()
                        .font_medium()
                        .child(format!("{value} pt")),
                ))
                .child(button(
                    "up",
                    IconName::Plus,
                    value < *SPACING_RANGE.end(),
                    value + SPACING_STEP,
                )),
        )
        .into_any_element()
}

fn choice<T: Copy + PartialEq + 'static>(
    state: &Entity<AppState>,
    id: &'static str,
    label: String,
    options: Vec<(T, String)>,
    value: T,
    set: fn(&mut MenuBarSettings, T),
) -> AnyElement {
    let state = state.clone();
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(div().text_sm().child(label.clone()))
        .child(ui::segmented(id, options, value, move |value, cx| {
            state.update(cx, |s, cx| {
                s.update_settings(|v| set(&mut v.menu_bar, value), cx)
            })
        }))
        .into_any_element()
}
