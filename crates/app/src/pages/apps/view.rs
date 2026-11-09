//! 应用页的画面：左边是应用列表，右边是选中应用的详情和它的残留。

use super::{AppsPage, Item, Leftovers, Row, Sort, tilde};
use crate::health::Level;
use crate::settings::SizeUnit;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, Hsla, Image, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    img, prelude::FluentBuilder as _, px, rems,
};
use pc_specs_core::apps;
use rust_i18n::t;
use std::cmp::Reverse;
use std::sync::Arc;

/// 这么多天没打开过，就提醒一句。
const IDLE_DAYS: u64 = 90;

/// 残留项连同它在列表里的下标（勾选时要用）。
type Indexed<'a> = Vec<(usize, &'a Item)>;

/// 本页用到的几个主题色，先取出来，省得到处借着 `cx`。
#[derive(Clone, Copy)]
struct Ink {
    border: Hsla,
    muted: Hsla,
    hover: Hsla,
    active: Hsla,
    radius: Pixels,
    radius_lg: Pixels,
}

pub(super) fn page(this: &AppsPage, cx: &Context<AppsPage>) -> AnyElement {
    let title = t!("nav_apps").into_owned();
    if !apps::SUPPORTED {
        return ui::page(
            "page-apps",
            title,
            None,
            vec![],
            vec![ui::empty_state(
                IconName::AppWindow,
                t!("apps_unsupported").into_owned(),
                "",
            )],
            cx,
        );
    }
    let entity = cx.entity();
    let theme = cx.theme();
    let ink = Ink {
        border: theme.border,
        muted: theme.muted_foreground,
        hover: theme.list_hover,
        active: theme.list_active,
        radius: theme.radius,
        radius_lg: theme.radius_lg,
    };
    let unit = this.state.read(cx).settings.size_unit;

    let refresh = {
        let entity = entity.clone();
        Button::new("apps-refresh")
            .small()
            .outline()
            .icon(IconName::RefreshCw)
            .label(t!("apps_refresh").into_owned())
            .disabled(this.loading || this.busy)
            .on_click(move |_, _, cx| entity.update(cx, |page, cx| page.refresh(cx)))
            .into_any_element()
    };
    let Some(rows) = &this.rows else {
        return ui::page(
            "page-apps",
            title,
            None,
            vec![refresh],
            ui::skeleton_body(),
            cx,
        );
    };

    let known: u64 = rows.iter().filter_map(|row| row.size).sum();
    let subtitle = (!rows.is_empty()).then(|| {
        SharedString::from(if this.loading {
            t!("apps_summary_measuring", n = rows.len()).into_owned()
        } else {
            t!(
                "apps_summary",
                n = rows.len(),
                size = fmt::bytes(known, unit)
            )
            .into_owned()
        })
    });

    // 大小还没量完时按大小排会一直跳：先按名字，量完再按大小。
    let sort = if this.loading && this.sort == Sort::Size {
        Sort::Name
    } else {
        this.sort
    };
    let query = this.search.read(cx).value().trim().to_lowercase();
    let mut shown: Vec<&Row> = rows
        .iter()
        .filter(|row| {
            query.is_empty()
                || row.app.name.to_lowercase().contains(&query)
                || row.app.id.to_lowercase().contains(&query)
        })
        .collect();
    match sort {
        Sort::Size => shown.sort_by_key(|row| Reverse(row.size.unwrap_or(0))),
        Sort::Unused => shown.sort_by_key(|row| row.app.last_used),
        // 核心库已经按名字排好了。
        Sort::Name => {}
    }

    let toolbar = {
        let sort_entity = entity.clone();
        h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                Input::new(&this.search)
                    .small()
                    .w(rems(16.))
                    .cleanable(true)
                    .prefix(Icon::new(IconName::Search).small()),
            )
            .child(ui::segmented(
                "apps-sort",
                vec![
                    (Sort::Size, t!("apps_sort_size").into_owned()),
                    (Sort::Name, t!("apps_sort_name").into_owned()),
                    (Sort::Unused, t!("apps_sort_unused").into_owned()),
                ],
                this.sort,
                move |sort, cx| {
                    sort_entity.update(cx, |page, cx| {
                        page.sort = sort;
                        cx.notify();
                    })
                },
            ))
    };

    let list = v_flex()
        .id("apps-list")
        .w(px(340.))
        .flex_shrink_0()
        .h_full()
        .min_h_0()
        .overflow_y_scrollbar()
        .gap_0p5()
        .pr_2()
        .when(shown.is_empty(), |this| {
            this.child(ui::muted(
                if rows.is_empty() {
                    t!("apps_empty").into_owned()
                } else {
                    t!("apps_no_match", query = query.as_str()).into_owned()
                },
                cx,
            ))
        })
        .children(shown.iter().map(|row| {
            list_row(
                &entity,
                row,
                this.selected.as_deref() == Some(row.key.as_str()),
                this.loading,
                unit,
                ink,
                cx,
            )
        }));

    let detail = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .rounded(ink.radius_lg)
        .border_1()
        .border_color(ink.border)
        .overflow_hidden()
        .child(match this.selected_row() {
            Some(row) => detail(this, row, &entity, unit, ink, cx),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::empty_state(
                    IconName::AppWindow,
                    t!("apps_pick").into_owned(),
                    "",
                ))
                .into_any_element(),
        });

    ui::page_fill(
        title,
        subtitle,
        vec![refresh],
        v_flex().size_full().gap_3().child(toolbar).child(
            h_flex()
                .flex_1()
                .min_h_0()
                .gap_4()
                .child(list)
                .child(detail),
        ),
        cx,
    )
}

/// 应用的图标；还没有（或者这个系统没有）就用一个通用的占位。
fn icon_view(icon: Option<Arc<Image>>, size: f32, ink: Ink) -> AnyElement {
    match icon {
        Some(image) => img(image).size(px(size)).flex_shrink_0().into_any_element(),
        None => div()
            .size(px(size))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(ink.radius)
            .bg(ink.hover)
            .child(Icon::new(IconName::AppWindow).text_color(ink.muted))
            .into_any_element(),
    }
}

fn list_row(
    entity: &Entity<AppsPage>,
    row: &Row,
    selected: bool,
    measuring: bool,
    unit: SizeUnit,
    ink: Ink,
    cx: &App,
) -> AnyElement {
    let size = match row.size {
        Some(bytes) => fmt::bytes(bytes, unit),
        None if measuring => "…".to_string(),
        None => "—".to_string(),
    };
    let detail = if row.app.version.is_empty() {
        size
    } else {
        format!("{size} · {}", row.app.version)
    };
    let (entity, key) = (entity.clone(), row.key.clone());
    h_flex()
        .id(SharedString::from(format!("apps-row-{}", row.key)))
        .w_full()
        .items_center()
        .gap_3()
        .px_2()
        .py_1p5()
        .rounded(ink.radius)
        .cursor_pointer()
        .when(selected, |this| this.bg(ink.active))
        .when(!selected, |this| {
            this.hover(move |style| style.bg(ink.hover))
        })
        .on_click(move |_, _, cx| entity.update(cx, |page, cx| page.select(key.clone(), cx)))
        .child(icon_view(row.icon.clone(), 32., ink))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .child(row.app.name.clone()),
                )
                .child(ui::muted(detail, cx).text_xs().truncate()),
        )
        .into_any_element()
}

/// 右边：应用本身的信息、残留文件，以及底下的操作栏。
fn detail(
    this: &AppsPage,
    row: &Row,
    entity: &Entity<AppsPage>,
    unit: SizeUnit,
    ink: Ink,
    cx: &App,
) -> AnyElement {
    let app = &row.app;

    // 版本、发布者、Bundle ID 合成一行。
    let mut meta: Vec<String> = Vec::new();
    if !app.version.is_empty() {
        meta.push(t!("apps_version", v = app.version.as_str()).into_owned());
    }
    if !app.publisher.is_empty() {
        meta.push(app.publisher.clone());
    }
    if !app.id.is_empty() && app.id != app.name {
        meta.push(app.id.clone());
    }

    let location = app.path.clone().map(|path| {
        let (text, reveal) = (tilde(&path), path.clone());
        h_flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(ink.muted)
                    .child(text),
            )
            .child(
                Button::new("apps-reveal")
                    .xsmall()
                    .ghost()
                    .icon(IconName::FolderSearch)
                    .tooltip(ui::reveal_label())
                    .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
            )
    });

    let header = h_flex()
        .w_full()
        .items_center()
        .gap_4()
        .child(icon_view(row.icon.clone(), 56., ink))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .truncate()
                        .text_xl()
                        .font_semibold()
                        .child(app.name.clone()),
                )
                .when(!meta.is_empty(), |this| {
                    this.child(ui::muted(meta.join(" · "), cx).text_xs().truncate())
                })
                .children(location),
        );

    // 占了多少、多久没打开了。
    let idle_days = app.last_used.map(|used| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        now.saturating_sub(used) / 86_400
    });
    let facts = h_flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .when_some(row.size, |this, bytes| {
            this.child(ui::chip(
                t!("apps_size", size = fmt::bytes(bytes, unit)).into_owned(),
            ))
        })
        .child(ui::chip(match app.last_used {
            Some(used) => t!("apps_last_used", date = date(used)).into_owned(),
            None => t!("apps_never_used").into_owned(),
        }))
        .when_some(idle_days.filter(|days| *days >= IDLE_DAYS), |this, days| {
            this.child(ui::level_tag(
                Level::Warning,
                t!("apps_idle", n = days / 30).into_owned(),
            ))
        });

    let mut sections: Vec<AnyElement> = vec![header.into_any_element(), facts.into_any_element()];
    sections.extend(leftover_sections(this, entity, unit, ink, cx));

    let (chosen, chosen_bytes) = this.chosen();
    // 残留还在找、或者上一次卸载还没做完的时候，按钮不给按：不然会漏掉后面才出现的残留。
    let locked = this.busy || matches!(this.leftovers, Leftovers::Loading);
    let total = row.size.unwrap_or(0) + chosen_bytes;
    let (uninstall, only_files) = (entity.clone(), entity.clone());
    let bar = h_flex()
        .w_full()
        .flex_shrink_0()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .px_4()
        .py_3()
        .border_t_1()
        .border_color(ink.border)
        .child(
            ui::muted(
                if apps::MOVES_TO_TRASH {
                    t!(
                        "apps_total",
                        n = chosen.len(),
                        size = fmt::bytes(total, unit)
                    )
                    .into_owned()
                } else {
                    String::new()
                },
                cx,
            )
            .text_xs(),
        )
        .child(
            h_flex()
                .gap_2()
                .when(apps::MOVES_TO_TRASH && !chosen.is_empty(), |this| {
                    this.child(
                        Button::new("apps-remove-files")
                            .small()
                            .outline()
                            .label(t!("apps_remove_files").into_owned())
                            .disabled(locked)
                            .on_click(move |_, window, cx| {
                                only_files.update(cx, |page, cx| page.ask(false, window, cx))
                            }),
                    )
                })
                .child(
                    Button::new("apps-uninstall")
                        .small()
                        .danger()
                        .icon(IconName::Trash)
                        .label(t!("apps_uninstall").into_owned())
                        .disabled(locked)
                        .on_click(move |_, window, cx| {
                            uninstall.update(cx, |page, cx| page.ask(true, window, cx))
                        }),
                ),
        );

    v_flex()
        .size_full()
        .child(
            v_flex()
                .id("apps-detail")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .gap_4()
                .p_4()
                .children(sections),
        )
        .child(bar)
        .into_any_element()
}

/// 残留文件：用户自己目录里的可以勾选，系统共用位置的只列出来。
fn leftover_sections(
    this: &AppsPage,
    entity: &Entity<AppsPage>,
    unit: SizeUnit,
    ink: Ink,
    cx: &App,
) -> Vec<AnyElement> {
    if !apps::MOVES_TO_TRASH {
        // Windows：程序自己的卸载程序会清理它自己的东西，这里不去找。
        return vec![
            ui::muted(t!("apps_win_note").into_owned(), cx)
                .text_xs()
                .into_any_element(),
        ];
    }
    let items = match &this.leftovers {
        Leftovers::None => return Vec::new(),
        Leftovers::Loading => {
            return vec![
                ui::muted(t!("apps_leftovers_loading").into_owned(), cx).into_any_element(),
            ];
        }
        Leftovers::Ready(items) => items,
    };
    if items.is_empty() {
        return vec![ui::muted(t!("apps_leftovers_none").into_owned(), cx).into_any_element()];
    }

    let (user, system): (Indexed, Indexed) = items
        .iter()
        .enumerate()
        .partition(|(_, item)| !item.leftover.system);
    let mut sections = Vec::new();

    if !user.is_empty() {
        let bytes: u64 = user.iter().map(|(_, item)| item.leftover.bytes).sum();
        let (all, none) = (entity.clone(), entity.clone());
        sections.push(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            v_flex()
                                .min_w_0()
                                .child(
                                    div().font_semibold().child(
                                        t!(
                                            "apps_leftovers_title",
                                            n = user.len(),
                                            size = fmt::bytes(bytes, unit)
                                        )
                                        .into_owned(),
                                    ),
                                )
                                .child(
                                    ui::muted(t!("apps_leftovers_desc").into_owned(), cx).text_xs(),
                                ),
                        )
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .gap_1()
                                .child(
                                    Button::new("apps-select-all")
                                        .xsmall()
                                        .ghost()
                                        .label(t!("apps_select_all").into_owned())
                                        .on_click(move |_, _, cx| {
                                            all.update(cx, |page, cx| {
                                                page.set_all_checked(true, cx)
                                            })
                                        }),
                                )
                                .child(
                                    Button::new("apps-select-none")
                                        .xsmall()
                                        .ghost()
                                        .label(t!("apps_select_none").into_owned())
                                        .on_click(move |_, _, cx| {
                                            none.update(cx, |page, cx| {
                                                page.set_all_checked(false, cx)
                                            })
                                        }),
                                ),
                        ),
                )
                .child(leftover_list(&user, entity, unit, ink))
                .into_any_element(),
        );
    }

    if !system.is_empty() {
        sections.push(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    v_flex()
                        .min_w_0()
                        .child(
                            div()
                                .font_semibold()
                                .child(t!("apps_system_title", n = system.len()).into_owned()),
                        )
                        .child(ui::muted(t!("apps_system_desc").into_owned(), cx).text_xs()),
                )
                .child(leftover_list(&system, entity, unit, ink))
                .into_any_element(),
        );
    }
    sections
}

fn leftover_list(
    items: &[(usize, &Item)],
    entity: &Entity<AppsPage>,
    unit: SizeUnit,
    ink: Ink,
) -> AnyElement {
    v_flex()
        .w_full()
        .px_3()
        .rounded(ink.radius_lg)
        .border_1()
        .border_color(ink.border)
        .children(items.iter().enumerate().map(|(position, (index, item))| {
            leftover_row(entity, *index, item, position + 1 == items.len(), unit, ink)
        }))
        .into_any_element()
}

fn leftover_row(
    entity: &Entity<AppsPage>,
    index: usize,
    item: &Item,
    last: bool,
    unit: SizeUnit,
    ink: Ink,
) -> AnyElement {
    let path = item.leftover.path.clone();
    let (full, reveal) = (path.display().to_string(), path.clone());
    // 只列出来的那些没有勾选框，留一格空位让路径对齐。
    let lead: AnyElement = if item.leftover.system {
        div().size(px(16.)).flex_shrink_0().into_any_element()
    } else {
        let entity = entity.clone();
        Checkbox::new(("apps-leftover", index))
            .checked(item.checked)
            .on_click(move |checked, _, cx| {
                let checked = *checked;
                entity.update(cx, |page, cx| page.set_checked(index, checked, cx));
            })
            .into_any_element()
    };
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .py_1()
        .when(!last, |this| this.border_b_1().border_color(ink.border))
        .child(lead)
        .child(
            div()
                .id(SharedString::from(format!("apps-leftover-path-{index}")))
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                .child(tilde(&path)),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_sm()
                .text_color(ink.muted)
                .child(fmt::bytes(item.leftover.bytes, unit)),
        )
        .child(
            Button::new(("apps-leftover-reveal", index))
                .xsmall()
                .ghost()
                .icon(IconName::FolderSearch)
                .tooltip(ui::reveal_label())
                .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
        )
        .into_any_element()
}

/// Unix 秒 → 本地日期 `YYYY-MM-DD`。
fn date(unix_secs: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix_secs as i64, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".into())
}
