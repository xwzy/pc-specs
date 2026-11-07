//! 大屏组件共用的部件：磁贴外框、数据行，以及两种版面——
//! 「仪表盘」（环形读数 + 数据行 + 趋势图，CPU / 内存 / GPU / 温度 / 电池用）和
//! 「速率」（两个大数字 + 双线趋势图，网络 / 磁盘读写用）。
//!
//! 所有尺寸都是「设计像素」：按一个 360×220 的单位格设计，乘上 `Ctx::k` 才是屏幕像素。
//! 磁贴越大，`Body`（可用的设计尺寸）越大，组件就摆出更多内容，而不是把字放大。

use super::config::{BigScreenSettings, WidgetKind};
use super::data::Point;
use super::layout::Rect;
use super::paint::{self, Line};
use super::theme::{ChartStyle, Palette};
use crate::menubar::data::{CELSIUS_FLOOR, CELSIUS_SPAN, RATE_FLOOR};
use crate::menubar::widgets::tabular;
use crate::state::AppState;
use gpui_kit::component::{Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, BoxShadow, Div, Hsla, IntoElement, ParentElement as _, Pixels, SharedString,
    Styled as _, div, hsla, prelude::FluentBuilder as _, px, relative,
};

/// 磁贴的内边距、标题行高度、标题与内容之间的距离。
pub const PAD: f32 = 14.;
pub const HEAD: f32 = 20.;
pub const GAP: f32 = 10.;
/// 一行数据（字号 12 + 行距）占的高度。
const ROW: f32 = 19.;

/// 从一条历史采样里取出某个指标。
type Pick = fn(&Point) -> Option<f32>;

/// 磁贴内容区的大小（设计像素）。
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub w: f32,
    pub h: f32,
}

/// 画一帧大屏用到的全部上下文。`k` 每个磁贴不同（见 `Ctx::scaled`）。
#[derive(Clone)]
pub struct Ctx<'a> {
    pub app: &'a AppState,
    pub cfg: &'a BigScreenSettings,
    pub pal: Palette,
    pub style: ChartStyle,
    /// 一个设计像素对应多少屏幕像素。
    pub k: f32,
    pub now: chrono::DateTime<chrono::Local>,
}

impl<'a> Ctx<'a> {
    pub fn scaled(&self, k: f32) -> Self {
        Self { k, ..self.clone() }
    }

    pub fn px(&self, design: f32) -> Pixels {
        px(design * self.k)
    }

    /// 一段文字（不换行）。
    pub fn text(&self, content: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
        div()
            .text_size(self.px(size))
            .line_height(self.px(size * 1.25))
            .text_color(color)
            .whitespace_nowrap()
            .child(content.into())
    }

    pub fn strong(&self, content: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
        self.text(content, size, color).font_semibold()
    }

    /// 等宽数字的粗体：数值每秒跳动时不会左右抖。
    pub fn number(&self, content: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
        tabular(self.strong(content, size, color))
    }

    /// 这张趋势图（宽 `width` 设计像素）切成多少个时间段：线 / 面积图每 3 个屏幕像素一段，
    /// 格栅图每根竖条 6.5 个；同时不能细过采样间隔，否则会出现空段。
    fn bins(&self, width: f32) -> usize {
        let per_bin = match self.style {
            ChartStyle::Bars => 6.5 * self.k.max(1.),
            _ => 3.,
        };
        let by_width = (width * self.k / per_bin) as usize;
        let interval = self.app.settings.interval_ms.max(250) as f32 * 1.15;
        let by_interval = (self.cfg.chart_window_ms() as f32 / interval) as usize;
        by_width.min(by_interval).clamp(8, 240)
    }

    fn values(
        &self,
        width: f32,
        pick: impl Fn(&Point) -> Option<f32>,
        normalize: impl Fn(f32) -> f32,
    ) -> Vec<Option<f32>> {
        self.app
            .wall
            .bins(self.cfg.chart_window_ms(), self.bins(width), pick)
            .into_iter()
            .map(|value| value.map(|v| normalize(v).clamp(0., 1.)))
            .collect()
    }

    /// 占用率（0–100）的趋势线。
    pub fn percent_line(
        &self,
        width: f32,
        color: Hsla,
        pick: impl Fn(&Point) -> Option<f32>,
    ) -> Line {
        Line {
            values: self.values(width, pick, |v| v / 100.),
            color,
        }
    }

    /// 摄氏度的趋势线，纵轴固定 20–100 °C（和菜单栏一致）。
    pub fn celsius_line(
        &self,
        width: f32,
        color: Hsla,
        pick: impl Fn(&Point) -> Option<f32>,
    ) -> Line {
        let (floor, span) = (CELSIUS_FLOOR as f32, CELSIUS_SPAN as f32);
        Line {
            values: self.values(width, pick, |v| (v - floor) / span),
            color,
        }
    }

    /// 两条速率趋势线共用一个量程；返回线和量程的峰值（字节每秒）。
    pub fn rate_lines(&self, width: f32, sides: [(Pick, Hsla); 2]) -> (Vec<Line>, f32) {
        let raw: Vec<Vec<Option<f32>>> = sides
            .iter()
            .map(|(pick, _)| {
                self.app
                    .wall
                    .bins(self.cfg.chart_window_ms(), self.bins(width), pick)
            })
            .collect();
        let peak = raw
            .iter()
            .flatten()
            .flatten()
            .copied()
            .fold(RATE_FLOOR as f32, f32::max);
        let lines = raw
            .into_iter()
            .zip(sides)
            .map(|(values, (_, color))| Line {
                values: values
                    .into_iter()
                    .map(|value| value.map(|v| (v / peak).clamp(0., 1.)))
                    .collect(),
                color,
            })
            .collect();
        (lines, peak)
    }
}

/// 每种组件的代表色。
pub fn accent(pal: &Palette, kind: WidgetKind) -> Hsla {
    match kind {
        WidgetKind::Cpu | WidgetKind::Processes => pal.cpu,
        WidgetKind::Memory => pal.mem,
        WidgetKind::Gpu => pal.gpu,
        WidgetKind::Network => pal.down,
        WidgetKind::DiskIo | WidgetKind::Storage => pal.disk,
        WidgetKind::Temperature => pal.temp,
        WidgetKind::Battery => pal.battery,
        WidgetKind::Clock | WidgetKind::System => pal.muted,
    }
}

/// 边框宽度只有整数像素的写法，按缩放取最接近的一档。
fn edged(frame: Div, width: f32) -> Div {
    match width.round() as i32 {
        ..=1 => frame.border_1(),
        2 => frame.border_2(),
        3 => frame.border_3(),
        _ => frame.border_4(),
    }
}

/// 磁贴：圆角外框 + 标题行 + 内容。`rect` 是屏幕像素，`body` 拿到的是内容区的设计尺寸。
pub fn tile(
    c: &Ctx,
    kind: WidgetKind,
    rect: Rect,
    aside: Option<String>,
    body: impl FnOnce(Body) -> AnyElement,
) -> AnyElement {
    let inner = Body {
        w: rect.w / c.k - 2. * PAD,
        h: rect.h / c.k - 2. * PAD - HEAD - GAP,
    };
    let accent = accent(&c.pal, kind);
    let mut frame = edged(div(), c.k * c.pal.edge_width)
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.w))
        .h(px(rect.h))
        .p(c.px(PAD))
        .overflow_hidden()
        .rounded(c.px(c.pal.radius))
        .bg(c.pal.tile)
        .border_color(c.pal.tile_edge);
    if c.pal.glow > 0. {
        frame = frame.shadow(vec![
            BoxShadow::new(px(0.), px(0.), accent.opacity(0.22 * c.pal.glow))
                .blur_radius(c.px(18.)),
        ]);
    } else if !c.pal.dark {
        // 浅色主题里磁贴是浮在底色上的白卡片，要一点柔和的投影。
        frame = frame.shadow(vec![
            BoxShadow::new(px(0.), c.px(4.), hsla(0., 0., 0., 0.08)).blur_radius(c.px(14.)),
        ]);
    }
    let header = h_flex()
        .flex_none()
        .h(c.px(HEAD))
        .items_center()
        .gap(c.px(7.))
        .child(
            Icon::new(kind.icon())
                .with_size(c.px(15.))
                .text_color(accent),
        )
        .child(c.strong(kind.title(), 13., c.pal.muted))
        .when_some(aside, |row, text| {
            row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .text_size(c.px(11.))
                    .text_color(c.pal.muted)
                    .child(text),
            )
        });
    frame
        .child(
            v_flex()
                .size_full()
                .gap(c.px(GAP))
                .child(header)
                .child(div().flex_1().min_h_0().w_full().child(body(inner))),
        )
        .into_any_element()
}

/// 内容放不下 / 没有数据时的占位。
pub fn empty(c: &Ctx, message: impl Into<SharedString>) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(c.text(message, 12., c.pal.muted))
        .into_any_element()
}

/// 一组「名称 …… 数值」的数据行，数值右对齐。
pub fn facts(c: &Ctx, rows: impl IntoIterator<Item = (String, String)>) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(c.px(4.))
        .children(rows.into_iter().map(|(label, value)| {
            h_flex()
                .w_full()
                .items_center()
                .gap(c.px(8.))
                .child(
                    c.text(label, 12., c.pal.muted)
                        .flex_none()
                        .max_w(relative(0.6)),
                )
                .child(tabular(
                    c.text(value, 12., c.pal.text)
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_right(),
                ))
        }))
}

/// 这么高的地方放得下几行数据。
pub fn rows_fitting(height: f32) -> usize {
    ((height + 4.) / ROW).floor().max(0.) as usize
}

/// 一条横向的进度条。
pub fn bar(c: &Ctx, fraction: f32, color: Hsla, height: f32) -> Div {
    div()
        .w_full()
        .h(c.px(height))
        .rounded_full()
        .bg(c.pal.faint)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(fraction.clamp(0., 1.)))
                .rounded_full()
                .bg(color),
        )
}

/// 趋势图：根据图表宽度（设计像素）生成线，返回（线，左上角的小字）。
pub type Chart<'a> = Box<dyn Fn(f32) -> (Vec<Line>, Option<String>) + 'a>;

fn chart_view(c: &Ctx, make: &Chart, width: f32) -> AnyElement {
    let (lines, label) = make(width);
    div()
        .relative()
        .size_full()
        .child(paint::chart(lines, c.style, c.pal.faint, c.k, c.pal.glow))
        .when_some(label, |view, label| {
            view.child(
                div()
                    .absolute()
                    .top(c.px(1.))
                    .left(c.px(3.))
                    .text_size(c.px(10.))
                    .text_color(c.pal.muted)
                    .child(label),
            )
        })
        .into_any_element()
}

// ---- 仪表盘版面 -----------------------------------------------------------------

/// 仪表盘版面要的内容。
pub struct Gauge<'a> {
    /// 环形进度（0–1）；`None` 是没有读数。
    pub fraction: Option<f32>,
    pub color: Hsla,
    /// 环中间的大字和它的单位。
    pub value: String,
    pub unit: String,
    /// 数据行，最重要的在前，放不下的从后面舍弃。
    pub facts: Vec<(String, String)>,
    pub chart: Option<Chart<'a>>,
    /// 每核心（或类似的）竖条：（0–1 的高度，颜色）。
    pub cores: Vec<(f32, Hsla)>,
}

fn ring_box(c: &Ctx, diameter: f32, gauge: &Gauge) -> Div {
    let chars = gauge.value.chars().count() as f32 + gauge.unit.chars().count() as f32 * 0.5;
    let size = (diameter * 0.30).min(diameter * 0.78 / (0.58 * chars.max(1.)));
    div()
        .relative()
        .flex_none()
        .size(c.px(diameter))
        .child(paint::ring(
            gauge.fraction,
            gauge.color,
            c.pal.faint,
            c.pal.glow,
        ))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    h_flex()
                        .items_baseline()
                        .gap(c.px(size * 0.1))
                        .child(c.number(gauge.value.clone(), size, c.pal.text))
                        .child(c.strong(gauge.unit.clone(), size * 0.42, c.pal.muted)),
                ),
        )
}

fn cores_strip(c: &Ctx, cores: &[(f32, Hsla)], height: f32) -> Div {
    div()
        .flex_none()
        .w_full()
        .h(c.px(height))
        .child(paint::columns(cores.to_vec(), c.pal.faint, c.k))
}

/// 趋势图（可选再加一排竖条）占满 `height` 设计像素高。
/// 两样都有但放不下时舍弃竖条；只有竖条时它占满整块。
fn lower_part(c: &Ctx, gauge: &Gauge, width: f32, height: f32) -> Div {
    let has_chart = gauge.chart.is_some();
    let show_cores = !gauge.cores.is_empty() && (!has_chart || height >= 80.);
    v_flex()
        .size_full()
        .gap(c.px(6.))
        .when_some(gauge.chart.as_ref(), |column, make| {
            column.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(chart_view(c, make, width)),
            )
        })
        .when(show_cores && has_chart, |column| {
            column.child(cores_strip(c, &gauge.cores, 22.))
        })
        .when(show_cores && !has_chart, |column| {
            column.child(div().flex_1().min_h_0().w_full().child(paint::columns(
                gauge.cores.clone(),
                c.pal.faint,
                c.k,
            )))
        })
}

/// 仪表盘版面。按内容区的形状在四种摆法里选一种：
/// 很扁（环 | 数据 | 趋势图并排）、横向（环和数据在上、趋势图在下）、竖向（环在上、依次往下）、
/// 极小（只有读数和趋势图）。
pub fn gauge_panel(c: &Ctx, b: Body, gauge: Gauge) -> AnyElement {
    let ratio = b.w / b.h;
    if b.h < 70. || b.w < 130. {
        compact(c, b, &gauge)
    } else if ratio >= 2.4 {
        wide(c, b, &gauge)
    } else if ratio >= 1.15 {
        landscape(c, b, &gauge)
    } else {
        portrait(c, b, &gauge)
    }
}

fn compact(c: &Ctx, b: Body, gauge: &Gauge) -> AnyElement {
    let size = (b.h * 0.6).clamp(18., 40.);
    let chars = gauge.value.chars().count() as f32 + gauge.unit.chars().count() as f32 * 0.5;
    let read_w = chars * size * 0.58 + GAP;
    let chart_w = b.w - read_w - GAP;
    h_flex()
        .size_full()
        .items_center()
        .gap(c.px(GAP))
        .child(
            h_flex()
                .flex_none()
                .items_baseline()
                .gap(c.px(3.))
                .child(c.number(gauge.value.clone(), size, gauge.color))
                .child(c.strong(gauge.unit.clone(), size * 0.45, c.pal.muted)),
        )
        .when(chart_w >= 70., |row| {
            row.when_some(gauge.chart.as_ref(), |row, make| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(chart_view(c, make, chart_w)),
                )
            })
        })
        .into_any_element()
}

fn wide(c: &Ctx, b: Body, gauge: &Gauge) -> AnyElement {
    let diameter = b.h.min(150.);
    let spare = b.w - diameter - GAP;
    let has_facts = !gauge.facts.is_empty();
    let has_chart = gauge.chart.is_some() || !gauge.cores.is_empty();
    let mut facts_w = if has_facts {
        (b.w * 0.26).clamp(120., 230.)
    } else {
        0.
    };
    let mut chart_w = spare - if has_facts { facts_w + GAP } else { 0. };
    if has_chart && chart_w < 90. {
        // 放不下趋势图：让数据行占满剩下的宽度。
        facts_w = spare;
        chart_w = 0.;
    }
    let show_chart = has_chart && chart_w > 0.;
    h_flex()
        .size_full()
        .items_center()
        .gap(c.px(GAP))
        .child(ring_box(c, diameter, gauge))
        .when(has_facts, |row| {
            let list = facts(
                c,
                gauge
                    .facts
                    .iter()
                    .take(rows_fitting(b.h))
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            if show_chart {
                row.child(div().flex_none().w(c.px(facts_w)).child(list))
            } else {
                row.child(div().flex_1().min_w_0().child(list))
            }
        })
        .when(show_chart, |row| {
            row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(lower_part(c, gauge, chart_w, b.h)),
            )
        })
        .into_any_element()
}

fn landscape(c: &Ctx, b: Body, gauge: &Gauge) -> AnyElement {
    let lower = gauge.chart.is_some() || !gauge.cores.is_empty();
    let diameter = if lower {
        (b.h * 0.56).min(b.w * 0.42).min(150.)
    } else {
        b.h.min(b.w * 0.45).min(170.)
    };
    let rest = b.h - diameter - GAP;
    let top = h_flex()
        .flex_none()
        .w_full()
        .h(c.px(diameter))
        .items_center()
        .gap(c.px(GAP + 4.))
        .child(ring_box(c, diameter, gauge))
        .child(
            div().flex_1().min_w_0().child(facts(
                c,
                gauge
                    .facts
                    .iter()
                    .take(rows_fitting(diameter))
                    .cloned()
                    .collect::<Vec<_>>(),
            )),
        );
    v_flex()
        .size_full()
        .gap(c.px(GAP))
        .when(!lower, |column| column.justify_center())
        .child(top)
        .when(lower && rest >= 34., |column| {
            column.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(lower_part(c, gauge, b.w, rest)),
            )
        })
        .into_any_element()
}

fn portrait(c: &Ctx, b: Body, gauge: &Gauge) -> AnyElement {
    let diameter = (b.w * 0.74).min(b.h * 0.40).min(180.);
    let rest = b.h - diameter - GAP;
    let lower = gauge.chart.is_some() || !gauge.cores.is_empty();
    let fact_rows = if lower {
        rows_fitting(rest * 0.45)
    } else {
        rows_fitting(rest)
    };
    let facts_h = fact_rows as f32 * ROW;
    let chart_h = rest - facts_h - GAP;
    v_flex()
        .size_full()
        .gap(c.px(GAP))
        .child(
            div()
                .flex_none()
                .w_full()
                .flex()
                .justify_center()
                .child(ring_box(c, diameter, gauge)),
        )
        .when(fact_rows > 0, |column| {
            column.child(facts(
                c,
                gauge
                    .facts
                    .iter()
                    .take(fact_rows)
                    .cloned()
                    .collect::<Vec<_>>(),
            ))
        })
        .when(lower && chart_h >= 40., |column| {
            column.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(lower_part(c, gauge, b.w, chart_h)),
            )
        })
        .into_any_element()
}

// ---- 速率版面 -------------------------------------------------------------------

/// 速率版面里的一侧（下载 / 上传，读 / 写）。
pub struct Side {
    pub arrow: &'static str,
    pub label: String,
    pub value: String,
    pub unit: String,
    pub color: Hsla,
}

pub struct Rates<'a> {
    pub sides: [Side; 2],
    /// 底部的说明行（接口名、累计流量……），放不下的从后面舍弃。
    pub footer: Vec<String>,
    pub chart: Chart<'a>,
}

fn readout(c: &Ctx, side: &Side, size: f32) -> Div {
    v_flex()
        .min_w_0()
        .gap(c.px(2.))
        .child(
            h_flex()
                .items_center()
                .gap(c.px(4.))
                .child(c.strong(side.arrow, 12., side.color))
                .child(c.text(side.label.clone(), 12., c.pal.muted)),
        )
        .child(
            h_flex()
                .items_baseline()
                .gap(c.px(size * 0.12))
                .child(c.number(side.value.clone(), size, side.color))
                .child(c.strong(side.unit.clone(), size * 0.42, c.pal.muted)),
        )
}

fn footer(c: &Ctx, lines: &[String], count: usize) -> Div {
    v_flex().flex_none().w_full().gap(c.px(2.)).children(
        lines
            .iter()
            .take(count)
            .map(|line| c.text(line.clone(), 11., c.pal.muted).w_full().truncate()),
    )
}

pub fn rate_panel(c: &Ctx, b: Body, rates: Rates) -> AnyElement {
    let ratio = b.w / b.h;
    if b.h < 62. {
        let size = (b.h * 0.5).clamp(14., 30.);
        return h_flex()
            .size_full()
            .items_center()
            .gap(c.px(GAP * 1.5))
            .children(rates.sides.iter().map(|side| {
                h_flex()
                    .items_baseline()
                    .gap(c.px(3.))
                    .child(c.strong(side.arrow, size * 0.55, side.color))
                    .child(c.number(side.value.clone(), size, side.color))
                    .child(c.strong(side.unit.clone(), size * 0.42, c.pal.muted))
            }))
            .into_any_element();
    }
    if ratio >= 2.1 {
        let column_w = (b.w * 0.36).clamp(130., 300.);
        let size = (b.h * 0.26).min(column_w / 5.).clamp(15., 36.);
        let lines = rows_fitting(b.h - 2. * (size * 2.4 + GAP));
        return h_flex()
            .size_full()
            .gap(c.px(GAP * 1.4))
            .child(
                v_flex()
                    .flex_none()
                    .w(c.px(column_w))
                    .h_full()
                    .justify_between()
                    .gap(c.px(GAP))
                    .child(
                        v_flex()
                            .gap(c.px(GAP))
                            .child(readout(c, &rates.sides[0], size))
                            .child(readout(c, &rates.sides[1], size)),
                    )
                    .child(footer(c, &rates.footer, lines.min(2))),
            )
            .child(div().flex_1().min_w_0().h_full().child(chart_view(
                c,
                &rates.chart,
                b.w - column_w - GAP * 1.4,
            )))
            .into_any_element();
    }
    let column_w = (b.w - GAP) / 2.;
    let size = (b.h * 0.2).min(column_w / 5.5).clamp(15., 34.);
    let top_h = size * 2.3 + 6.;
    let rest = b.h - top_h - GAP;
    let lines = if rest >= 100. {
        2
    } else if rest >= 70. {
        1
    } else {
        0
    };
    let chart_h = rest
        - if lines > 0 {
            lines as f32 * 15. + GAP
        } else {
            0.
        };
    v_flex()
        .size_full()
        .gap(c.px(GAP))
        .child(
            h_flex()
                .flex_none()
                .gap(c.px(GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(readout(c, &rates.sides[0], size)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(readout(c, &rates.sides[1], size)),
                ),
        )
        .when(chart_h >= 30., |column| {
            column.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(chart_view(c, &rates.chart, b.w)),
            )
        })
        .when(lines > 0, |column| {
            column.child(footer(c, &rates.footer, lines))
        })
        .into_any_element()
}
