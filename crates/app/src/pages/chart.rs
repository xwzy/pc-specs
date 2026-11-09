//! 图表数据与样式的共用部分。
//!
//! 注意：同一个调用点会创建多张图表时（比如本模块里的函数），必须显式给每张图一个 `id`，
//! 否则它们共享默认 id（取自调用位置），悬停状态与路径缓存会互相串扰。

use crate::fmt;
use crate::state::AppState;
use gpui_kit::component::{ActiveTheme as _, chart::AreaChart, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Hsla, IntoElement as _, ParentElement as _, SharedString, Styled as _, div, px,
};
use pc_specs_core::model::MonitorTick;

/// 单序列图表的一个点。
#[derive(Clone)]
pub struct Point {
    pub label: SharedString,
    pub value: f64,
}

/// 双序列图表的一个点（下载 / 上传、读 / 写）。
#[derive(Clone)]
pub struct Point2 {
    pub label: SharedString,
    pub a: f64,
    pub b: f64,
}

/// 最近 `last` 个采样（`None` 表示全部）里取一个值。
pub fn series(s: &AppState, last: Option<usize>, f: impl Fn(&MonitorTick) -> f64) -> Vec<Point> {
    let skip = last.map_or(0, |n| s.ticks.len().saturating_sub(n));
    s.ticks
        .iter()
        .zip(s.tick_labels.iter())
        .skip(skip)
        .map(|(tick, label)| Point {
            label: label.clone(),
            value: f(tick),
        })
        .collect()
}

pub fn series2(
    s: &AppState,
    last: Option<usize>,
    f: impl Fn(&MonitorTick) -> (f64, f64),
) -> Vec<Point2> {
    let skip = last.map_or(0, |n| s.ticks.len().saturating_sub(n));
    s.ticks
        .iter()
        .zip(s.tick_labels.iter())
        .skip(skip)
        .map(|(tick, label)| {
            let (a, b) = f(tick);
            Point2 {
                label: label.clone(),
                a,
                b,
            }
        })
        .collect()
}

/// 卡片里的迷你走势：无坐标轴、无网格、不响应鼠标。
pub fn sparkline(
    id: &str,
    points: Vec<Point>,
    domain: Option<(f64, f64)>,
    color: Hsla,
) -> AnyElement {
    let mut chart = AreaChart::<Point, SharedString, f64>::new(points)
        .id(SharedString::from(id.to_string()))
        .x(|d| d.label.clone())
        .y(|d| d.value)
        .stroke(color)
        .fill(color.opacity(0.18))
        .linear()
        .x_axis(false)
        .grid(false)
        .interactive(false)
        .appear(false);
    if let Some((lo, hi)) = domain {
        chart = chart.y_domain(lo, hi);
    }
    chart.into_any_element()
}

/// 悬停提示里的数值：位数沿用纵轴的规则，再带上单位（`65.8%`、`3.20 MB/s`）。
/// 默认的提示会直接打出 `65.7766342163086` 这样的原始浮点数。
fn tooltip_text(value: f64, unit: &str) -> SharedString {
    let number = fmt::axis_label(value);
    // 百分号和度数紧贴数字，其余单位前留一个空格。
    if unit.starts_with('%') || unit.starts_with('°') {
        format!("{number}{unit}").into()
    } else {
        format!("{number} {unit}").into()
    }
}

/// 带坐标轴与悬停提示的单序列面积图。`unit` 只用于悬停提示里的数值。
pub fn area(
    id: &str,
    points: Vec<Point>,
    domain: Option<(f64, f64)>,
    color: Hsla,
    y_format: impl Fn(f64) -> String + 'static,
    unit: &'static str,
) -> AnyElement {
    let mut chart = AreaChart::<Point, SharedString, f64>::new(points)
        .id(SharedString::from(id.to_string()))
        .x(|d| d.label.clone())
        .y(|d| d.value)
        .stroke(color)
        .fill(color.opacity(0.18))
        .linear()
        .x_tick_count(4)
        .y_axis(true)
        .y_tick_count(3)
        .y_tick_format(y_format)
        .tooltip_value(move |_, _, value| tooltip_text(value, unit))
        .appear(false);
    if let Some((lo, hi)) = domain {
        // 固定量程（0–100%）时不留顶部余量，否则最上面一格会标成 108% 之类。
        chart = chart.y_domain(lo, hi).y_padding(0.0, 0.0);
    }
    chart.into_any_element()
}

/// 双序列面积图。`names` 同时用于悬停提示和图下方的图例，让人一眼分清两条线。
/// `unit` 只用于悬停提示里的数值。
pub fn area2(
    id: &str,
    points: Vec<Point2>,
    colors: (Hsla, Hsla),
    names: (String, String),
    y_format: impl Fn(f64) -> String + 'static,
    unit: &'static str,
    cx: &App,
) -> AnyElement {
    let legend = h_flex()
        .gap_4()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .children(
            [(colors.0, names.0.clone()), (colors.1, names.1.clone())]
                .into_iter()
                .map(|(color, name)| {
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(div().flex_none().size(px(8.)).rounded_full().bg(color))
                        .child(name)
                }),
        );
    let chart = AreaChart::<Point2, SharedString, f64>::new(points)
        .id(SharedString::from(id.to_string()))
        .x(|d| d.label.clone())
        .y(|d| d.a)
        .stroke(colors.0)
        .fill(colors.0.opacity(0.18))
        .name(names.0)
        .y(|d| d.b)
        .stroke(colors.1)
        .fill(colors.1.opacity(0.18))
        .name(names.1)
        .linear()
        .x_tick_count(4)
        .y_axis(true)
        .y_tick_count(3)
        .y_tick_format(y_format)
        .tooltip_value(move |_, _, value| tooltip_text(value, unit))
        .appear(false);
    v_flex()
        .size_full()
        .gap_1()
        .child(div().flex_1().min_h_0().w_full().child(chart))
        .child(legend)
        .into_any_element()
}

/// 供各页面取图表配色。主题自带的 chart_1..5 是同一色系的深浅，双序列图里分不清，
/// 所以直接取语义色：蓝、绿、青、洋红、黄。
pub fn palette(cx: &App) -> [Hsla; 5] {
    let t = cx.theme();
    [t.blue, t.green, t.cyan, t.magenta, t.yellow]
}
