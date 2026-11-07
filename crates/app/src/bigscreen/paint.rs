//! 大屏用到的几种画法：环形仪表、趋势图（面积 / 折线 / 格栅）、竖条、背景网格。
//! 全部用 canvas 直接画，不依赖图表库；尺寸由外层的布局决定，这里只管填满给定的矩形。

use super::theme::{ChartStyle, Grid};
use gpui_kit::{
    Bounds, Hsla, IntoElement, PathBuilder, Styled as _, Window, canvas, fill, linear_color_stop,
    linear_gradient, point, px, size,
};
use std::f32::consts::{FRAC_PI_2, TAU};

fn dot(window: &mut Window, x: f32, y: f32, radius: f32, color: Hsla) {
    window.paint_quad(
        fill(
            Bounds::new(
                point(px(x - radius), px(y - radius)),
                size(px(radius * 2.), px(radius * 2.)),
            ),
            color,
        )
        .corner_radii(px(radius)),
    );
}

fn rect(window: &mut Window, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Hsla) {
    if w <= 0. || h <= 0. {
        return;
    }
    window.paint_quad(
        fill(Bounds::new(point(px(x), px(y)), size(px(w), px(h))), color)
            .corner_radii(px(radius.min(w / 2.).min(h / 2.))),
    );
}

fn stroke_circle(window: &mut Window, cx: f32, cy: f32, radius: f32, width: f32, color: Hsla) {
    let mut path = PathBuilder::stroke(px(width));
    let radii = point(px(radius), px(radius));
    path.move_to(point(px(cx + radius), px(cy)));
    path.arc_to(radii, px(0.), false, true, point(px(cx - radius), px(cy)));
    path.arc_to(radii, px(0.), false, true, point(px(cx + radius), px(cy)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// 从正上方开始顺时针画 `fraction`（0–1）圈。
fn stroke_arc(
    window: &mut Window,
    cx: f32,
    cy: f32,
    radius: f32,
    width: f32,
    fraction: f32,
    color: Hsla,
) {
    if fraction >= 0.999 {
        stroke_circle(window, cx, cy, radius, width, color);
        return;
    }
    let end = -FRAC_PI_2 + TAU * fraction;
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(point(px(cx), px(cy - radius)));
    path.arc_to(
        point(px(radius), px(radius)),
        px(0.),
        fraction > 0.5,
        true,
        point(px(cx + radius * end.cos()), px(cy + radius * end.sin())),
    );
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// 环形仪表：灰色的轨道加一段圆头的进度弧。`fraction` 是 `None` 时只画轨道。
/// `glow` 大于 0 时，进度弧外面再叠两层淡淡的光晕。
pub fn ring(fraction: Option<f32>, color: Hsla, track: Hsla, glow: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let diameter = bounds.size.width.as_f32().min(bounds.size.height.as_f32());
            if diameter < 6. {
                return;
            }
            let thickness = (diameter * 0.1).max(2.);
            let radius = (diameter - thickness) / 2.;
            let center = bounds.center();
            let (cx, cy) = (center.x.as_f32(), center.y.as_f32());
            stroke_circle(window, cx, cy, radius, thickness, track);

            let Some(fraction) = fraction.map(|f| f.clamp(0., 1.)) else {
                return;
            };
            if fraction < 0.004 {
                return;
            }
            if glow > 0. {
                stroke_arc(
                    window,
                    cx,
                    cy,
                    radius,
                    thickness * 2.4,
                    fraction,
                    color.opacity(0.09 * glow),
                );
                stroke_arc(
                    window,
                    cx,
                    cy,
                    radius,
                    thickness * 1.6,
                    fraction,
                    color.opacity(0.15 * glow),
                );
            }
            stroke_arc(window, cx, cy, radius, thickness, fraction, color);
            // 圆头：路径本身是平头的。
            let end = -FRAC_PI_2 + TAU * fraction;
            dot(window, cx, cy - radius, thickness / 2., color);
            dot(
                window,
                cx + radius * end.cos(),
                cy + radius * end.sin(),
                thickness / 2.,
                color,
            );
        },
    )
    .size_full()
}

/// 趋势图里的一条线。`values` 已经归一化到 0–1，最新的在最右，`None` 是没有数据的时间段。
pub struct Line {
    pub values: Vec<Option<f32>>,
    pub color: Hsla,
}

/// 沿着 `points` 描一条平滑的曲线（Catmull-Rom 转成三次贝塞尔），控制点限制在 `lo..=hi` 之内，
/// 免得曲线冲出图表的上下边。
fn trace(path: &mut PathBuilder, points: &[(f32, f32)], lo: f32, hi: f32) {
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        let a = (
            p1.0 + (p2.0 - p0.0) / 6.,
            (p1.1 + (p2.1 - p0.1) / 6.).clamp(lo, hi),
        );
        let b = (
            p2.0 - (p3.0 - p1.0) / 6.,
            (p2.1 - (p3.1 - p1.1) / 6.).clamp(lo, hi),
        );
        path.cubic_bezier_to(
            point(px(p2.0), px(p2.1)),
            point(px(a.0), px(a.1)),
            point(px(b.0), px(b.1)),
        );
    }
}

/// 趋势图。`unit` 是设计尺寸到屏幕像素的缩放，线宽、圆点都跟着它；`glow` 大于 0 时线条带光晕。
pub fn chart(
    lines: Vec<Line>,
    style: ChartStyle,
    guide: Hsla,
    unit: f32,
    glow: f32,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (x0, y0) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
            let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            if w < 8. || h < 8. {
                return;
            }
            let hairline = unit.max(1.);
            if style != ChartStyle::Bars {
                for step in 0..4 {
                    let y = y0 + h - h * step as f32 / 4. - if step == 0 { hairline } else { 0. };
                    rect(window, x0, y, w, hairline, 0., guide);
                }
            }
            match style {
                ChartStyle::Bars => bars(window, &lines, (x0, y0, w, h), guide, unit),
                ChartStyle::Area | ChartStyle::Line => {
                    for line in &lines {
                        curve(
                            window,
                            line,
                            (x0, y0, w, h),
                            style == ChartStyle::Area,
                            unit,
                            glow,
                        );
                    }
                }
            }
        },
    )
    .size_full()
}

fn curve(
    window: &mut Window,
    line: &Line,
    (x0, y0, w, h): (f32, f32, f32, f32),
    area: bool,
    unit: f32,
    glow: f32,
) {
    let stroke = (unit * 2.).max(1.5);
    let (top, bottom) = (y0 + stroke / 2. + unit * 2., y0 + h - stroke / 2.);
    let count = line.values.len();
    if count == 0 || bottom <= top {
        return;
    }
    let x_at = |i: usize| {
        if count == 1 {
            x0 + w / 2.
        } else {
            x0 + w * i as f32 / (count - 1) as f32
        }
    };
    let y_at = |value: f32| bottom - value.clamp(0., 1.) * (bottom - top);

    let mut last = None;
    let mut index = 0;
    while index < count {
        if line.values[index].is_none() {
            index += 1;
            continue;
        }
        let end = line.values[index..]
            .iter()
            .position(Option::is_none)
            .map_or(count, |offset| index + offset);
        let points: Vec<(f32, f32)> = (index..end)
            .filter_map(|i| line.values[i].map(|value| (x_at(i), y_at(value))))
            .collect();
        index = end;
        let Some(&tip) = points.last() else { continue };
        last = Some(tip);
        if points.len() < 2 {
            dot(window, tip.0, tip.1, stroke, line.color);
            continue;
        }
        if area {
            let mut fill_path = PathBuilder::fill();
            fill_path.move_to(point(px(points[0].0), px(y0 + h)));
            fill_path.line_to(point(px(points[0].0), px(points[0].1)));
            trace(&mut fill_path, &points, top, bottom);
            fill_path.line_to(point(px(tip.0), px(y0 + h)));
            fill_path.close();
            if let Ok(path) = fill_path.build() {
                window.paint_path(
                    path,
                    linear_gradient(
                        180.,
                        linear_color_stop(line.color.opacity(0.34), 0.),
                        linear_color_stop(line.color.opacity(0.02), 1.),
                    ),
                );
            }
        }
        for (width, alpha) in [(stroke * 3.2, 0.07 * glow), (stroke * 1.9, 0.14 * glow)] {
            if glow > 0. {
                paint_line(
                    window,
                    &points,
                    width,
                    top,
                    bottom,
                    line.color.opacity(alpha),
                );
            }
        }
        paint_line(window, &points, stroke, top, bottom, line.color);
    }
    if let Some((x, y)) = last {
        // 最新一个点（只在贴着右边缘时才标出来：数据已经断了很久的线不该再冒出一个「当前值」）。
        if x >= x0 + w - w / count.max(2) as f32 * 2. {
            dot(window, x, y, stroke * 2.6, line.color.opacity(0.22));
            dot(window, x, y, stroke * 1.25, line.color);
        }
    }
}

fn paint_line(
    window: &mut Window,
    points: &[(f32, f32)],
    width: f32,
    lo: f32,
    hi: f32,
    color: Hsla,
) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(point(px(points[0].0), px(points[0].1)));
    trace(&mut path, points, lo, hi);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// 格栅图：每个时间段一根竖条，越新越实，像示波器 / 终端里的音量表。
fn bars(
    window: &mut Window,
    lines: &[Line],
    (x0, y0, w, h): (f32, f32, f32, f32),
    track: Hsla,
    unit: f32,
) {
    let count = lines
        .iter()
        .map(|line| line.values.len())
        .max()
        .unwrap_or(0);
    if count == 0 {
        return;
    }
    let slot = w / count as f32;
    let gap = (slot * 0.28).max(unit.min(slot / 3.));
    let bar = (slot - gap).max(1.);
    let sub = bar / lines.len().max(1) as f32;
    let radius = unit * 1.5;
    for i in 0..count {
        let x = x0 + i as f32 * slot + gap / 2.;
        rect(window, x, y0, bar, h, radius, track);
        let age = if count > 1 {
            i as f32 / (count - 1) as f32
        } else {
            1.
        };
        for (j, line) in lines.iter().enumerate() {
            let Some(value) = line.values.get(i).copied().flatten() else {
                continue;
            };
            let value = value.clamp(0., 1.);
            if value < 0.004 {
                continue;
            }
            let height = (value * h).max(unit * 1.5).min(h);
            rect(
                window,
                x + sub * j as f32,
                y0 + h - height,
                sub,
                height,
                radius,
                line.color.opacity(0.35 + 0.65 * age),
            );
        }
    }
}

/// 一排竖条（每个 CPU 核心一根）：灰色轨道里从下往上填。每项是（0–1 的高度，颜色）。
pub fn columns(items: Vec<(f32, Hsla)>, track: Hsla, unit: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (x0, y0) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
            let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            if items.is_empty() || w < 4. || h < 4. {
                return;
            }
            let slot = w / items.len() as f32;
            let gap = (slot * 0.3).max(unit.min(slot / 3.));
            let bar = (slot - gap).max(1.);
            for (i, (value, color)) in items.iter().enumerate() {
                let x = x0 + i as f32 * slot + gap / 2.;
                rect(window, x, y0, bar, h, bar / 2.5, track);
                let value = value.clamp(0., 1.);
                if value > 0.004 {
                    let height = (value * h).max(unit * 1.5).min(h);
                    rect(window, x, y0 + h - height, bar, height, bar / 2.5, *color);
                }
            }
        },
    )
    .size_full()
}

/// 背景上的装饰网格 / 点阵，铺满整个窗口、放在所有内容下面。`unit` 是设计尺寸的缩放。
pub fn backdrop(grid: Grid, color: Hsla, unit: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (x0, y0) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
            let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            match grid {
                Grid::None => {}
                Grid::Lines(step) => {
                    let step = (step * unit).max(8.);
                    let mut x = step;
                    while x < w {
                        rect(window, x0 + x, y0, 1., h, 0., color);
                        x += step;
                    }
                    let mut y = step;
                    while y < h {
                        rect(window, x0, y0 + y, w, 1., 0., color);
                        y += step;
                    }
                }
                Grid::Dots(step) => {
                    // 点太密画起来就是白白浪费：总数封顶。
                    let mut step = (step * unit).max(8.);
                    while (w / step) * (h / step) > 6000. {
                        step *= 1.25;
                    }
                    let radius = (unit * 1.1).max(0.8);
                    let mut y = step / 2.;
                    while y < h {
                        let mut x = step / 2.;
                        while x < w {
                            dot(window, x0 + x, y0 + y, radius, color);
                            x += step;
                        }
                        y += step;
                    }
                }
            }
        },
    )
    .absolute()
    .size_full()
}
