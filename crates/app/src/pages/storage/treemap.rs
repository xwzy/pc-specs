//! 矩形树图：把一棵目录树按大小排成一块块矩形，目录里再嵌着它的子项。
//!
//! 排版用 squarified 算法（Bruls、Huizing、van Wijk，2000）：每一块尽量接近正方形，
//! 大小一眼可比。排版只取决于树和画布尺寸，与鼠标无关，悬停和选中都不会触发重排。
//! 画法是一整块 canvas：色块和文字都直接画在上面，不会冒出成百上千个小元素。

use crate::fmt;
use crate::settings::SizeUnit;
use gpui_kit::{
    App, Bounds, FontWeight, Hsla, ShapedLine, SharedString, TextAlign, TextRun, TruncateFrom,
    Window, fill, hsla, linear_color_stop, linear_gradient, point, px, size,
};
use pc_specs_core::space::{CATEGORIES, Category, Cats, Kind, Node};
use std::cmp::Reverse;
use std::sync::Arc;

/// 比这个边长（px）还小的块不画：肉眼分不出来，只会白白增加绘制量。
const MIN_TILE: f32 = 3.0;
/// 目录框顶部标题条的高度（px）。
const HEADER: f32 = 17.0;
/// 目录展开成框至少要这么大；再小就画成一整块，免得框里只剩一圈边。
const FRAME_MIN_W: f32 = 52.0;
const FRAME_MIN_H: f32 = 34.0;
/// 目录框带标题条至少要这么大。
const HEADER_MIN_W: f32 = 72.0;
const HEADER_MIN_H: f32 = HEADER + 24.0;
/// 目录框内侧的留白（px）。
const PAD: f32 = 2.0;
/// 一次排版最多产生多少块，防止极端的树把界面拖慢。
const MAX_TILES: usize = 6000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn right(self) -> f32 {
        self.x + self.w
    }

    fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// 四条边各自取整：相邻的两块共用同一条边，既不会露缝，也不会叠在一起。
    fn snapped(self) -> Rect {
        let (x, y) = (self.x.round(), self.y.round());
        Rect {
            x,
            y,
            w: self.right().round() - x,
            h: self.bottom().round() - y,
        }
    }

    fn inset(self, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            x: self.x + left,
            y: self.y + top,
            w: self.w - left - right,
            h: self.h - top - bottom,
        }
    }
}

/// 把 `rect` 按 `weights` 切成一块块，面积与权重成正比。
/// 要求权重从大到小排好、且都大于 0；返回的矩形与权重一一对应。
pub fn squarify(weights: &[f64], rect: Rect) -> Vec<Rect> {
    let mut out = vec![Rect::default(); weights.len()];
    let total: f64 = weights.iter().sum();
    if total <= 0.0 || rect.w < 1.0 || rect.h < 1.0 {
        return out;
    }
    let scale = f64::from(rect.w) * f64::from(rect.h) / total;
    let areas: Vec<f64> = weights.iter().map(|weight| weight * scale).collect();

    let (mut x, mut y) = (f64::from(rect.x), f64::from(rect.y));
    let (mut w, mut h) = (f64::from(rect.w), f64::from(rect.h));
    let mut start = 0;
    while start < areas.len() {
        let side = w.min(h);
        if side <= 0.0 {
            break;
        }
        // 沿着剩余区域的短边摆一排，直到再放一块会让这一排里最差的长宽比变坏。
        let (mut end, mut sum, mut lo, mut hi, mut worst) =
            (start, 0.0, f64::MAX, 0.0f64, f64::MAX);
        while end < areas.len() {
            let area = areas[end];
            let (next_sum, next_lo, next_hi) = (sum + area, lo.min(area), hi.max(area));
            let next_worst = (side * side * next_hi / (next_sum * next_sum))
                .max(next_sum * next_sum / (side * side * next_lo));
            if end > start && next_worst > worst {
                break;
            }
            (sum, lo, hi, worst) = (next_sum, next_lo, next_hi, next_worst);
            end += 1;
        }

        // 这一排占满短边，厚度 = 面积之和 / 短边。
        let thickness = sum / side;
        let row = areas[start..end].iter().zip(&mut out[start..end]);
        if w >= h {
            let mut top = y;
            for (area, slot) in row {
                let height = area / thickness;
                *slot = Rect {
                    x: x as f32,
                    y: top as f32,
                    w: thickness as f32,
                    h: height as f32,
                };
                top += height;
            }
            x += thickness;
            w -= thickness;
        } else {
            let mut left = x;
            for (area, slot) in row {
                let width = area / thickness;
                *slot = Rect {
                    x: left as f32,
                    y: y as f32,
                    w: width as f32,
                    h: thickness as f32,
                };
                left += width;
            }
            y += thickness;
            h -= thickness;
        }
        start = end;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    File,
    /// 没有展开的目录（太小，或者层数用完了）：画成整块。
    Dir,
    /// 展开的目录：自己是一个框，里面是它的子项。
    Frame,
    /// 被扫描器裁掉的小项目合并成的一块。
    Rest,
}

/// 排好版的一块。
pub struct Tile {
    /// 相对整棵树的根的下标路径（不是相对当前视图）。
    pub path: Vec<u32>,
    /// 相对画布左上角。
    pub rect: Rect,
    pub shape: Shape,
    pub name: SharedString,
    pub size: u64,
    pub category: Category,
    /// 目录 / 小项目里各类型的字节数；文件没有。
    cats: Option<Box<Cats>>,
    /// 目录框顶部标题条的高度；0 表示没有标题条。
    pub header: f32,
}

impl Tile {
    /// 这一块里有多大比例属于 `category`（0..=1）。
    pub fn share(&self, category: Category) -> f32 {
        if self.size == 0 {
            return 0.0;
        }
        let bytes = match &self.cats {
            Some(cats) => cats[category.index()],
            None if self.category == category => self.size,
            None => 0,
        };
        (bytes as f64 / self.size as f64) as f32
    }
}

#[derive(Default)]
pub struct Layout {
    /// 父在子之前：先画的在下面。
    pub tiles: Vec<Tile>,
}

impl Layout {
    /// 点 `(x, y)` 下最里面的那一块。
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.tiles.iter().rposition(|tile| tile.rect.contains(x, y))
    }

    pub fn find(&self, path: &[u32]) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.path == path)
    }
}

/// 把 `root` 下 `view` 指的那个目录排进 `width` × `height` 的画布。
/// `levels` 是最多展开几层目录：1 表示只画这个目录的直接子项，不再往里嵌。
pub fn layout(root: &Node, view: &[u32], width: f32, height: f32, levels: u8) -> Layout {
    let mut tiles = Vec::new();
    if let Some(node) = root.at(view) {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: width.floor(),
            h: height.floor(),
        };
        lay(node, &mut view.to_vec(), area, 1, levels, &mut tiles);
    }
    Layout { tiles }
}

fn lay(node: &Node, path: &mut Vec<u32>, area: Rect, depth: u8, levels: u8, out: &mut Vec<Tile>) {
    // 删掉一项之后子项不会重排，所以这里自己按大小排；体积为 0 的没有面积，不参与。
    let mut order: Vec<usize> = (0..node.children.len())
        .filter(|&i| node.children[i].size > 0)
        .collect();
    order.sort_by_key(|&i| Reverse(node.children[i].size));
    let weights: Vec<f64> = order
        .iter()
        .map(|&i| node.children[i].size as f64)
        .collect();

    for (&index, rect) in order.iter().zip(squarify(&weights, area)) {
        if out.len() >= MAX_TILES {
            return;
        }
        let rect = rect.snapped();
        if rect.w < MIN_TILE || rect.h < MIN_TILE {
            continue;
        }
        let child = &node.children[index];
        path.push(index as u32);

        let expand = child.kind == Kind::Dir
            && depth < levels
            && !child.children.is_empty()
            && rect.w >= FRAME_MIN_W
            && rect.h >= FRAME_MIN_H;
        let (shape, header) = match child.kind {
            Kind::File => (Shape::File, 0.0),
            Kind::Rest => (Shape::Rest, 0.0),
            Kind::Dir if expand => {
                let titled = rect.w >= HEADER_MIN_W && rect.h >= HEADER_MIN_H;
                (Shape::Frame, if titled { HEADER } else { 0.0 })
            }
            Kind::Dir => (Shape::Dir, 0.0),
        };
        out.push(Tile {
            path: path.clone(),
            rect,
            shape,
            name: clean(&child.name),
            size: child.size,
            category: child.category,
            cats: child.cats.clone(),
            header,
        });

        if shape == Shape::Frame {
            let inner = rect.inset(PAD, header.max(PAD), PAD, PAD);
            if inner.w >= MIN_TILE && inner.h >= MIN_TILE {
                lay(child, path, inner, depth + 1, levels, out);
            }
        }
        path.pop();
    }
}

/// 文件名里偶尔有换行、制表符；画文字的时候不允许出现换行。
fn clean(name: &str) -> SharedString {
    if name.contains(char::is_control) {
        name.replace(char::is_control, " ").into()
    } else {
        name.to_string().into()
    }
}

// ---- 配色 -------------------------------------------------------------------

/// 每种类型一个色相（0..1）。八种颜色在色环上尽量拉开；「其它」是偏灰的蓝。
const HUES: [f32; CATEGORIES] = [0.97, 0.76, 0.36, 0.60, 0.08, 0.50, 0.13, 0.62];

/// 类型的代表色：图例、占比条和色块用同一套，一眼对得上。
pub fn category_color(category: Category, dark: bool) -> Hsla {
    let hue = HUES[category.index()];
    match (category, dark) {
        (Category::Other, true) => hsla(hue, 0.12, 0.42, 1.0),
        (Category::Other, false) => hsla(hue, 0.12, 0.52, 1.0),
        (_, true) => hsla(hue, 0.52, 0.40, 1.0),
        (_, false) => hsla(hue, 0.58, 0.46, 1.0),
    }
}

/// 目录框的底色与标题条：同一色相，但很淡，让里面的色块跳出来。
fn frame_color(category: Category, dark: bool) -> (Hsla, Hsla) {
    let hue = HUES[category.index()];
    let saturation = if category == Category::Other {
        0.10
    } else {
        0.30
    };
    if dark {
        (
            hsla(hue, saturation, 0.15, 1.0),
            hsla(hue, saturation + 0.04, 0.23, 1.0),
        )
    } else {
        (
            hsla(hue, saturation, 0.93, 1.0),
            hsla(hue, saturation + 0.04, 0.85, 1.0),
        )
    }
}

/// 画布用到的几个主题色。
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// 画布底色，同时也是色块之间那条细缝的颜色。
    pub surface: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
}

/// 一帧要画的东西。
pub struct Scene {
    pub layout: Arc<Layout>,
    /// 悬停 / 选中的块在 `layout.tiles` 里的下标。
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    /// 只突出这个类型，其余的淡出。
    pub filter: Option<Category>,
    pub unit: SizeUnit,
    pub palette: Palette,
    /// 被裁掉的小项目那一块上写的字。
    pub rest_label: SharedString,
}

fn clear() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.0)
}

/// 在画布 `bounds` 里画出整张树图。
pub fn paint(scene: &Scene, bounds: Bounds<gpui_kit::Pixels>, window: &mut Window, cx: &mut App) {
    let palette = scene.palette;
    let at = |rect: Rect| {
        Bounds::new(
            point(bounds.origin.x + px(rect.x), bounds.origin.y + px(rect.y)),
            size(px(rect.w), px(rect.h)),
        )
    };
    window.paint_quad(fill(bounds, palette.surface));

    for tile in &scene.layout.tiles {
        let rect = tile.rect;
        match tile.shape {
            Shape::Frame => {
                let (body, header) = frame_color(tile.category, palette.dark);
                window.paint_quad(
                    fill(at(rect), body)
                        .border_widths(px(1.0))
                        .border_color(palette.surface),
                );
                if tile.header > 0.0 {
                    let strip = Rect {
                        h: tile.header,
                        ..rect
                    }
                    .inset(1.0, 1.0, 1.0, 0.0);
                    window.paint_quad(fill(at(strip), header));
                    let size_text = fmt::bytes(tile.size, scene.unit);
                    let room = rect.w - 10.0;
                    let size_line = (rect.w >= 130.0)
                        .then(|| {
                            shape(
                                window,
                                &size_text,
                                10.5,
                                palette.muted,
                                FontWeight::NORMAL,
                                room * 0.5,
                            )
                        })
                        .flatten();
                    let size_width = size_line
                        .as_ref()
                        .map_or(0.0, |line| line.width.as_f32() + 8.0);
                    let name_line = shape(
                        window,
                        &tile.name,
                        11.0,
                        palette.text,
                        FontWeight::SEMIBOLD,
                        room - size_width,
                    );
                    let top = rect.y + (tile.header - 14.0) / 2.0;
                    if let Some(line) = name_line {
                        draw(window, cx, &line, bounds, rect.x + 5.0, top);
                    }
                    if let Some(line) = size_line {
                        let x = rect.right() - 5.0 - line.width.as_f32();
                        draw(window, cx, &line, bounds, x, top);
                    }
                }
            }
            Shape::File | Shape::Dir | Shape::Rest => {
                let base = if tile.shape == Shape::Rest {
                    if palette.dark {
                        hsla(0.6, 0.06, 0.30, 1.0)
                    } else {
                        hsla(0.6, 0.06, 0.62, 1.0)
                    }
                } else {
                    category_color(tile.category, palette.dark)
                };
                let (top, bottom) = (
                    hsla(base.h, base.s, (base.l + 0.05).min(1.0), 1.0),
                    hsla(base.h, base.s, (base.l - 0.03).max(0.0), 1.0),
                );
                window.paint_quad(
                    fill(
                        at(rect),
                        linear_gradient(
                            180.0,
                            linear_color_stop(top, 0.0),
                            linear_color_stop(bottom, 1.0),
                        ),
                    )
                    .border_widths(px(1.0))
                    .border_color(palette.surface),
                );
                // 没展开的目录比文件多一圈浅色内框，一眼能和文件区分开。
                if tile.shape == Shape::Dir && rect.w >= 8.0 && rect.h >= 8.0 {
                    window.paint_quad(
                        fill(at(rect.inset(1.0, 1.0, 1.0, 1.0)), clear())
                            .border_widths(px(1.0))
                            .border_color(hsla(0.0, 0.0, 1.0, 0.24)),
                    );
                }

                if rect.w >= 40.0 && rect.h >= 18.0 {
                    let name: &str = if tile.shape == Shape::Rest {
                        &scene.rest_label
                    } else {
                        &tile.name
                    };
                    let room = rect.w - 10.0;
                    if let Some(line) = shape(
                        window,
                        name,
                        11.0,
                        hsla(0.0, 0.0, 1.0, 0.96),
                        FontWeight::MEDIUM,
                        room,
                    ) {
                        draw(window, cx, &line, bounds, rect.x + 5.0, rect.y + 3.0);
                    }
                    if rect.h >= 34.0
                        && let Some(line) = shape(
                            window,
                            &fmt::bytes(tile.size, scene.unit),
                            10.5,
                            hsla(0.0, 0.0, 1.0, 0.72),
                            FontWeight::NORMAL,
                            room,
                        )
                    {
                        draw(window, cx, &line, bounds, rect.x + 5.0, rect.y + 18.0);
                    }
                }

                if let Some(category) = scene.filter {
                    let fade = (1.0 - tile.share(category)) * 0.8;
                    if fade > 0.02 {
                        window.paint_quad(fill(at(rect), palette.surface.opacity(fade)));
                    }
                }
            }
        }
    }

    let outline = |window: &mut Window, index: usize, width: f32, color: Hsla| {
        if let Some(tile) = scene.layout.tiles.get(index) {
            window.paint_quad(
                fill(at(tile.rect), clear())
                    .border_widths(px(width))
                    .border_color(color),
            );
        }
    };
    if let Some(index) = scene.hover.filter(|&index| Some(index) != scene.selected) {
        outline(window, index, 2.0, palette.text.opacity(0.7));
    }
    if let Some(index) = scene.selected {
        outline(window, index, 3.0, palette.text);
    }
}

/// 排好一行文字；放不下就在末尾截断加省略号，连一个字都放不下就不画。
pub(super) fn shape(
    window: &Window,
    text: &str,
    font_size: f32,
    color: Hsla,
    weight: FontWeight,
    max_width: f32,
) -> Option<ShapedLine> {
    if text.is_empty() || max_width < font_size {
        return None;
    }
    let mut font = window.text_style().font();
    font.weight = weight;
    let font_size = px(font_size);
    let text = SharedString::from(text.to_string());
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = std::slice::from_ref(&run);
    let line = window
        .text_system()
        .shape_line(text.clone(), font_size, runs, None);
    if line.width <= px(max_width) {
        return Some(line);
    }
    let mut wrapper = window.text_system().line_wrapper(font, font_size);
    let (shown, runs) = wrapper.truncate_line(text, px(max_width), "…", runs, TruncateFrom::End);
    drop(wrapper);
    Some(
        window
            .text_system()
            .shape_line(shown, font_size, &runs, None),
    )
}

pub(super) fn draw(
    window: &mut Window,
    cx: &mut App,
    line: &ShapedLine,
    bounds: Bounds<gpui_kit::Pixels>,
    x: f32,
    y: f32,
) {
    let origin = point(bounds.origin.x + px(x), bounds.origin.y + px(y));
    let _ = line.paint(origin, px(14.0), TextAlign::Left, None, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pc_specs_core::space::{Options, Progress, Tree, scan_with};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    fn area(rect: Rect) -> f32 {
        rect.w * rect.h
    }

    fn overlap(a: Rect, b: Rect) -> f32 {
        let w = a.right().min(b.right()) - a.x.max(b.x);
        let h = a.bottom().min(b.bottom()) - a.y.max(b.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.right() <= outer.right()
            && inner.bottom() <= outer.bottom()
    }

    fn aspect(rect: Rect) -> f32 {
        rect.w.max(rect.h) / rect.w.min(rect.h)
    }

    #[test]
    fn squarify_fills_the_rect_in_proportion() {
        let weights = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let whole = Rect {
            x: 10.0,
            y: 20.0,
            w: 600.0,
            h: 400.0,
        };
        let rects = squarify(&weights, whole);
        assert_eq!(rects.len(), weights.len());

        let total: f64 = weights.iter().sum();
        let mut covered = 0.0;
        for (weight, rect) in weights.iter().zip(&rects) {
            assert!(inside(*rect, whole), "{rect:?} sticks out of {whole:?}");
            let expect = (weight / total) as f32 * area(whole);
            assert!((area(*rect) - expect).abs() < 1.0, "{rect:?} vs {expect}");
            assert!(aspect(*rect) < 3.5, "{rect:?} is too elongated");
            covered += area(*rect);
        }
        assert!((covered - area(whole)).abs() < 1.0);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(overlap(*a, *b) < 0.5, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn squarify_survives_lopsided_and_degenerate_input() {
        let whole = Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 200.0,
        };
        assert!(squarify(&[], whole).is_empty());
        assert_eq!(squarify(&[5.0], whole), vec![whole]);
        // 一个独占 99.99%、一个千分之一：不能出现 NaN，也不能跑出画布。
        for rect in squarify(&[1e12, 1e9, 3.0, 1.0], whole) {
            assert!(rect.x.is_finite() && rect.w.is_finite() && rect.h.is_finite());
            assert!(inside(rect, whole));
        }
        // 画布退化成一条线：什么也不排，也不 panic。
        let flat = Rect { h: 0.0, ..whole };
        assert!(squarify(&[2.0, 1.0], flat).iter().all(|r| area(*r) == 0.0));
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7u8; bytes]).unwrap();
    }

    /// 在临时目录里造一棵真实的树再扫描：这样测到的就是扫描器真正产出的结构。
    fn sample_tree(tag: &str) -> Tree {
        let dir =
            std::env::temp_dir().join(format!("pc-specs-treemap-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("a/a1.bin"), 200 * 1024);
        write(&dir.join("a/a2.bin"), 100 * 1024);
        write(&dir.join("a/a3.bin"), 50 * 1024);
        // b 超过 1 MiB，扫描器才会给它保留各类型的明细。
        write(&dir.join("b/sub/s1.mp4"), 900 * 1024);
        write(&dir.join("b/sub/s2.mp4"), 60 * 1024);
        write(&dir.join("b/b1.txt"), 300 * 1024);
        write(&dir.join("c.bin"), 400 * 1024);
        write(&dir.join("d.txt"), 40 * 1024);
        let options = Options {
            base_keep: 0,
            max_children: 400,
            budget: 1_000_000,
            threads: 2,
        };
        let tree = scan_with(
            &dir,
            &options,
            &Progress::default(),
            &AtomicBool::new(false),
        )
        .expect("scan the sample tree");
        let _ = fs::remove_dir_all(&dir);
        tree
    }

    #[test]
    fn layout_nests_directories_without_overlap() {
        let tree = sample_tree("nest");
        let (width, height) = (800.0, 500.0);
        let laid = layout(&tree.root, &[], width, height, 3);
        let canvas = Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
        };
        assert!(!laid.tiles.is_empty());

        for tile in &laid.tiles {
            assert!(inside(tile.rect, canvas), "{:?}", tile.rect);
            assert!(tile.rect.w >= MIN_TILE && tile.rect.h >= MIN_TILE);
            if tile.path.len() > 1 {
                // 每一块都躺在它上一级的目录框里面。
                let parent = &tile.path[..tile.path.len() - 1];
                let frame = laid
                    .tiles
                    .iter()
                    .find(|t| t.path == parent)
                    .expect("a nested tile always has its frame");
                assert_eq!(frame.shape, Shape::Frame);
                assert!(inside(tile.rect, frame.rect));
            }
        }
        // 同一级的兄弟之间互不重叠。
        for (i, a) in laid.tiles.iter().enumerate() {
            for b in &laid.tiles[i + 1..] {
                if a.path[..a.path.len() - 1] == b.path[..b.path.len() - 1] {
                    assert_eq!(overlap(a.rect, b.rect), 0.0, "{:?} vs {:?}", a.path, b.path);
                }
            }
        }
        // 顶层几块把整个画布铺满（取整会有几个像素的出入），而且最大的文件面积最大。
        let top: Vec<&Tile> = laid.tiles.iter().filter(|t| t.path.len() == 1).collect();
        let covered: f32 = top.iter().map(|t| area(t.rect)).sum();
        assert!((covered - width * height).abs() / (width * height) < 0.01);
        let biggest = top.iter().max_by_key(|t| t.size).unwrap();
        assert!(top.iter().all(|t| area(t.rect) <= area(biggest.rect) + 1.0));
        assert!(laid.tiles.iter().any(|t| t.shape == Shape::Frame));
    }

    #[test]
    fn layout_levels_limit_how_deep_it_nests() {
        let tree = sample_tree("levels");
        let flat = layout(&tree.root, &[], 800.0, 500.0, 1);
        assert!(
            flat.tiles
                .iter()
                .all(|t| t.path.len() == 1 && t.shape != Shape::Frame)
        );
        let deep = layout(&tree.root, &[], 800.0, 500.0, 4);
        assert!(
            deep.tiles.iter().any(|t| t.path.len() == 3),
            "b/sub/* should be laid out"
        );
        assert!(deep.tiles.len() > flat.tiles.len());
    }

    #[test]
    fn hit_finds_the_innermost_tile() {
        let tree = sample_tree("hit");
        let laid = layout(&tree.root, &[], 800.0, 500.0, 3);
        let deepest = laid
            .tiles
            .iter()
            .enumerate()
            .max_by_key(|(_, t)| t.path.len())
            .map(|(i, _)| i)
            .unwrap();
        let rect = laid.tiles[deepest].rect;
        let found = laid.hit(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        assert_eq!(found, Some(deepest));
        assert_eq!(laid.find(&laid.tiles[deepest].path), Some(deepest));
        assert_eq!(laid.hit(-5.0, 10.0), None);
        assert_eq!(laid.find(&[99]), None);
    }

    #[test]
    fn layout_of_a_missing_view_is_empty() {
        let tree = sample_tree("missing");
        assert!(
            layout(&tree.root, &[99, 1], 800.0, 500.0, 3)
                .tiles
                .is_empty()
        );
    }

    #[test]
    fn share_reports_the_part_of_a_tile_in_a_category() {
        let tree = sample_tree("share");
        let laid = layout(&tree.root, &[], 800.0, 500.0, 1);
        let tile = |name: &str| laid.tiles.iter().find(|t| &*t.name == name).unwrap();
        // 文件：整块都是自己的类型。
        let c = tile("c.bin");
        assert_eq!(c.share(Category::Program), 1.0);
        assert_eq!(c.share(Category::Video), 0.0);
        // 带明细的目录：按各类型的字节数分。
        let b = tile("b");
        let (video, document) = (b.share(Category::Video), b.share(Category::Document));
        assert!(video > 0.7 && document > 0.2, "{video} {document}");
        assert!((video + document - 1.0).abs() < 1e-3);
        // 没有明细的小目录：整块算它占空间最多的那一种。
        assert_eq!(tile("a").share(Category::Program), 1.0);
    }
}
