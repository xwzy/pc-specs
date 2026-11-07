//! 各页面共用的小部件。保持薄：只封装重复出现的组合，不做额外抽象。

use crate::health::{self, Level};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonGroup},
    description_list::DescriptionList,
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle},
    group_box::{GroupBox, GroupBoxVariants as _},
    h_flex,
    progress::Progress,
    scroll::ScrollableElement as _,
    skeleton::Skeleton,
    table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div, prelude::FluentBuilder as _, rems,
};

fn page_header(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    actions: Vec<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap_4()
        .px_6()
        .pt_5()
        .pb_3()
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(div().text_xl().font_semibold().child(title.into()))
                .when_some(subtitle, |this, subtitle| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(subtitle),
                    )
                }),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .children(actions),
        )
}

/// 页面骨架：标题区 + 可滚动正文。`id` 决定滚动位置的归属，每个页面各自独立。
pub fn page(
    id: &'static str,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    actions: Vec<AnyElement>,
    body: Vec<AnyElement>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .size_full()
        .min_h_0()
        .child(page_header(title, subtitle, actions, cx))
        .child(
            v_flex()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .items_stretch()
                .gap_4()
                .px_6()
                .pb_6()
                .children(body),
        )
        .into_any_element()
}

/// 页面骨架：标题区 + 撑满剩余高度的正文。正文自己负责滚动（比如虚拟列表表格）。
pub fn page_fill(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    actions: Vec<AnyElement>,
    body: impl IntoElement,
    cx: &App,
) -> AnyElement {
    v_flex()
        .size_full()
        .min_h_0()
        .child(page_header(title, subtitle, actions, cx))
        .child(div().flex_1().min_h_0().px_6().pb_6().child(body))
        .into_any_element()
}

/// 带标题的内容区块（填充底色）。
pub fn section(id: impl Into<ElementId>, title: impl Into<SharedString>) -> GroupBox {
    GroupBox::new().id(id).title(title.into()).fill()
}

/// 无底色的分组标题 + 内容，用于直接放置带边框的列表或表格。
pub fn plain_section(id: impl Into<ElementId>, title: impl Into<SharedString>) -> GroupBox {
    GroupBox::new().id(id).title(title.into())
}

/// 规格键值列表（两列）。值为空时显示「—」。
pub fn spec_list(items: Vec<(String, String)>) -> DescriptionList {
    let mut list = DescriptionList::new().columns(2);
    for (label, value) in items {
        let value = if value.trim().is_empty() {
            "—".to_string()
        } else {
            value
        };
        list = list.item(label, value, 1);
    }
    list
}

pub fn level_color(level: Level, cx: &App) -> Hsla {
    match level {
        Level::Healthy => cx.theme().success,
        Level::Warning => cx.theme().warning,
        Level::Critical => cx.theme().danger,
    }
}

/// 占用率颜色：<75% 主色，75–90% 警告，>=90% 危险。
pub fn usage_color(percent: f32, cx: &App) -> Hsla {
    match health::usage_level(percent) {
        Level::Healthy => cx.theme().primary,
        Level::Warning => cx.theme().warning,
        Level::Critical => cx.theme().danger,
    }
}

pub fn usage_bar(id: impl Into<ElementId>, percent: f32, cx: &App) -> Progress {
    Progress::new(id)
        .value(percent)
        .color(usage_color(percent, cx))
}

/// 「标签 ……… 数值」+ 占用条，用于内存 / 显存 / 磁盘等容量类指标。
pub fn meter(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    percent: f32,
    cx: &App,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(label.into()),
                )
                .child(div().child(value.into())),
        )
        .child(usage_bar(id, percent, cx))
        .into_any_element()
}

pub fn level_tag(level: Level, text: impl Into<SharedString>) -> Tag {
    match level {
        Level::Healthy => Tag::success(),
        Level::Warning => Tag::warning(),
        Level::Critical => Tag::danger(),
    }
    .child(text.into())
}

/// 次要标签（类型、状态等）。
pub fn chip(text: impl Into<SharedString>) -> Tag {
    Tag::secondary().small().child(text.into())
}

pub fn muted(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// 空状态：图标 + 标题 + 说明。
pub fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> AnyElement {
    Empty::new()
        .header(
            EmptyHeader::new()
                .media(
                    EmptyMedia::new()
                        .with_variant(EmptyMediaVariant::Icon)
                        .child(Icon::new(icon)),
                )
                .title(EmptyTitle::new().child(title.into()))
                .description(EmptyDescription::new().child(description.into())),
        )
        .into_any_element()
}

/// 模块数据尚未到达时的骨架屏。
pub fn skeleton_body() -> Vec<AnyElement> {
    vec![
        Skeleton::new()
            .h_24()
            .w_full()
            .rounded_lg()
            .into_any_element(),
        Skeleton::new()
            .h_40()
            .w_full()
            .rounded_lg()
            .into_any_element(),
        Skeleton::new()
            .h_24()
            .w_full()
            .rounded_lg()
            .into_any_element(),
    ]
}

/// 分段选择器（单选）。`options` 的顺序即按钮顺序。
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<SharedString>,
    options: Vec<(T, String)>,
    current: T,
    on_select: impl Fn(T, &mut App) + 'static,
) -> ButtonGroup {
    let id = id.into();
    let values: Vec<T> = options.iter().map(|(value, _)| *value).collect();
    ButtonGroup::new(id.clone())
        .outline()
        .small()
        .children(options.into_iter().enumerate().map(|(i, (value, label))| {
            Button::new(SharedString::from(format!("{id}-{i}")))
                .label(label)
                .selected(value == current)
        }))
        .on_click(move |clicked, _, cx| {
            if let Some(value) = clicked.first().and_then(|i| values.get(*i)) {
                on_select(*value, cx);
            }
        })
}

/// 静态表格的列定义。`width` 单位是 rem，`None` 表示按剩余空间均分。
pub struct Col {
    title: SharedString,
    right: bool,
    width: Option<f32>,
}

impl Col {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            right: false,
            width: None,
        }
    }

    pub fn right(mut self) -> Self {
        self.right = true;
        self
    }

    pub fn width(mut self, rems: f32) -> Self {
        self.width = Some(rems);
        self
    }
}

/// 纯文本单元格。
pub fn txt(text: impl Into<SharedString>) -> AnyElement {
    div().truncate().child(text.into()).into_any_element()
}

/// 紧凑的静态表格。列最小宽度压到 4rem，避免窄窗口下被裁掉。
pub fn table(cols: Vec<Col>, rows: Vec<Vec<AnyElement>>) -> Table {
    let heads: Vec<TableHead> = cols
        .iter()
        .map(|col| {
            let mut head = TableHead::new().min_w(rems(4.));
            if col.right {
                head = head.text_right();
            }
            if let Some(w) = col.width {
                head = head.w(rems(w)).flex_none();
            }
            head.child(col.title.clone())
        })
        .collect();
    let body: Vec<TableRow> = rows
        .into_iter()
        .map(|cells| {
            TableRow::new().children(cells.into_iter().zip(&cols).map(|(cell, col)| {
                let mut c = TableCell::new().min_w(rems(4.));
                if col.right {
                    c = c.text_right();
                }
                if let Some(w) = col.width {
                    c = c.w(rems(w)).flex_none();
                }
                c.child(cell)
            }))
        })
        .collect();
    Table::new()
        .small()
        .child(TableHeader::new().child(TableRow::new().children(heads)))
        .child(TableBody::new().children(body))
}

/// 表格单元格里的次要文字。
pub fn dim(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// 带图标的小标签行：`[icon] label`。
pub fn icon_label(icon: IconName, text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    h_flex()
        .items_center()
        .gap_1p5()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon).small())
        .child(div().text_sm().child(text.into()))
}
