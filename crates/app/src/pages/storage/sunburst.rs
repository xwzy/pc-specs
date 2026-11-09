//! 旋转图（sunburst）：同心圆环，圆心是正在看的目录，往外一圈圈展开它的子目录；
//! 弧越长占的空间越多。和树图看的是同一棵树，只是换一种更容易看出「层级」的摆法。
//! 画法和树图一样是一整块 canvas：弧形事先排好、铺成路径缓存起来，悬停和选中只是在上面叠一层。

use super::treemap::{Palette, category_color, draw, shape};
use crate::fmt;
use crate::settings::SizeUnit;
use gpui_kit::{
    App, Bounds, FontWeight, Hsla, Path, PathBuilder, Pixels, Point, SharedString, Window, fill,
    hsla, point, px,
};
use pc_specs_core::space::{Category, Cats, Kind, Node};
use std::cmp::Reverse;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

/// 弧线外沿上不到这么长（px）的块不画：肉眼分不出来，只会白白增加绘制量。
const MIN_ARC: f32 = 2.0;
/// 一次排版最多产生多少块弧，防止极端的树把界面拖慢。
const MAX_SLICES: usize = 3000;
/// 圆环离画布边缘的留白（px）。
const MARGIN: f32 = 6.0;
/// 相邻两块之间的缝（px）。
const GAP: f32 = 0.6;

/// 排好版的一块弧。
pub struct Slice {
    /// 相对整棵树的根的下标路径（不是相对当前视图）。
    pub path: Vec<u32>,
    /// 起止角度（弧度）：从正上方起，顺时针量。
    from: f32,
    to: f32,
    inner: f32,
    outer: f32,
    /// 第几圈，从 1 数起。
    depth: u8,
    pub name: SharedString,
    pub size: u64,
    pub category: Category,
    pub kind: Kind,
    cats: Option<Box<Cats>>,
    /// 弧的形状，坐标相对画布左上角。
    outline: Path<Pixels>,
}

impl Slice {
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

    /// 弧形中心点（相对画布），标签画在这里。
    fn centroid(&self, center: (f32, f32)) -> (f32, f32) {
        let (radius, angle) = ((self.inner + self.outer) / 2.0, (self.from + self.to) / 2.0);
        (
            center.0 + radius * angle.sin(),
            center.1 - radius * angle.cos(),
        )
    }

    /// 标签能占多宽、多高：圆环在这个位置上水平、竖直方向各有多少空间。
    fn room(&self) -> (f32, f32) {
        let mid = (self.inner + self.outer) / 2.0;
        let thickness = self.outer - self.inner;
        let length = (self.to - self.from) * mid;
        let angle = (self.from + self.to) / 2.0;
        let (sin, cos) = (angle.sin().abs(), angle.cos().abs());
        (
            thickness * sin + length * cos,
            thickness * cos + length * sin,
        )
    }
}

#[derive(Default)]
pub struct Rings {
    /// 里圈在前：先画的在下面。
    pub slices: Vec<Slice>,
    center: (f32, f32),
    /// 中间空出来的圆的半径。
    hole: f32,
}

impl Rings {
    /// 点 `(x, y)` 下的那一块；圆心的空洞和圆环外面都没有。
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let (dx, dy) = (x - self.center.0, y - self.center.1);
        let radius = dx.hypot(dy);
        if radius < self.hole {
            return None;
        }
        // 从正上方起顺时针量：和排版时的角度同一个方向。
        let mut angle = dx.atan2(-dy);
        if angle < 0.0 {
            angle += TAU;
        }
        self.slices.iter().position(|slice| {
            radius >= slice.inner && radius < slice.outer && angle >= slice.from && angle < slice.to
        })
    }

    /// 点是不是落在圆心的空洞里（点一下就回到上一级）。
    pub fn in_hole(&self, x: f32, y: f32) -> bool {
        self.hole > 0.0 && (x - self.center.0).hypot(y - self.center.1) < self.hole
    }

    pub fn find(&self, path: &[u32]) -> Option<usize> {
        self.slices.iter().position(|slice| slice.path == path)
    }
}

struct Ctx {
    center: (f32, f32),
    hole: f32,
    ring: f32,
    levels: u8,
}

/// 把 `root` 下 `view` 指的那个目录排成 `width` × `height` 画布里的同心圆环。
/// `levels` 是最多画几圈。
pub fn layout(root: &Node, view: &[u32], width: f32, height: f32, levels: u8) -> Rings {
    let Some(node) = root.at(view) else {
        return Rings::default();
    };
    let radius = width.min(height) / 2.0 - MARGIN;
    if radius < 40.0 || levels == 0 {
        return Rings::default();
    }
    let hole = (radius * 0.22).clamp(34.0, 84.0);
    let ctx = Ctx {
        center: (width / 2.0, height / 2.0),
        hole,
        ring: (radius - hole) / f32::from(levels),
        levels,
    };
    let mut slices = Vec::new();
    lay(node, &mut view.to_vec(), 0.0, TAU, 1, &ctx, &mut slices);
    Rings {
        slices,
        center: ctx.center,
        hole,
    }
}

fn lay(
    node: &Node,
    path: &mut Vec<u32>,
    from: f32,
    to: f32,
    depth: u8,
    ctx: &Ctx,
    out: &mut Vec<Slice>,
) {
    // 删掉一项之后子项不会重排，所以这里自己按大小排；体积为 0 的没有弧长，不参与。
    let mut order: Vec<usize> = (0..node.children.len())
        .filter(|&i| node.children[i].size > 0)
        .collect();
    order.sort_by_key(|&i| Reverse(node.children[i].size));
    let sum: f64 = order.iter().map(|&i| node.children[i].size as f64).sum();
    let inner = ctx.hole + f32::from(depth - 1) * ctx.ring;
    let outer = inner + ctx.ring;

    let mut angle = from;
    for &index in &order {
        let child = &node.children[index];
        let start = angle;
        angle += (to - from) * (child.size as f64 / sum) as f32;
        if (angle - start) * ((inner + outer) / 2.0) < MIN_ARC {
            continue;
        }
        if out.len() >= MAX_SLICES {
            return;
        }
        path.push(index as u32);
        out.push(Slice {
            path: path.clone(),
            from: start,
            to: angle,
            inner,
            outer,
            depth,
            name: clean(&child.name),
            size: child.size,
            category: child.category,
            kind: child.kind,
            cats: child.cats.clone(),
            outline: outline(ctx.center, inner, outer, start, angle),
        });
        if child.kind == Kind::Dir && depth < ctx.levels && !child.children.is_empty() {
            lay(child, path, start, angle, depth + 1, ctx, out);
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

/// 在 `path` 里描出一块环形扇面。画布坐标 y 向下，所以角度增大就是顺时针（SVG 的 sweep）。
fn trace(path: &mut PathBuilder, center: (f32, f32), inner: f32, outer: f32, from: f32, to: f32) {
    let at = |radius: f32, angle: f32| {
        point(
            px(center.0 + radius * angle.sin()),
            px(center.1 - radius * angle.cos()),
        )
    };
    let radii = |radius: f32| point(px(radius), px(radius));
    if to - from >= TAU - 1e-3 {
        // 整整一圈：起点和终点重合的弧没有确定的方向，拆成两个半圆；内外两个圆按奇偶规则挖出圆环。
        let half = from + PI;
        for (radius, sweep) in [(outer, true), (inner, false)] {
            path.move_to(at(radius, from));
            path.arc_to(radii(radius), px(0.), false, sweep, at(radius, half));
            path.arc_to(radii(radius), px(0.), false, sweep, at(radius, from));
            path.close();
        }
        return;
    }
    let large = to - from > PI;
    path.move_to(at(outer, from));
    path.arc_to(radii(outer), px(0.), large, true, at(outer, to));
    path.line_to(at(inner, to));
    path.arc_to(radii(inner), px(0.), large, false, at(inner, from));
    path.close();
}

/// 一块弧的填充形状，四周各缩进一点点，相邻的块之间留出一道缝。
fn outline(center: (f32, f32), inner: f32, outer: f32, from: f32, to: f32) -> Path<Pixels> {
    let middle = (inner + outer) / 2.0;
    let gap = GAP / middle;
    let (from, to) = if to - from > gap * 3.0 && to - from < TAU - 1e-3 {
        (from + gap, to - gap)
    } else {
        (from, to)
    };
    let mut path = PathBuilder::fill();
    trace(&mut path, center, inner + GAP, outer - GAP, from, to);
    path.build().unwrap_or_else(|_| Path::new(Point::default()))
}

/// 把缓存的形状平移到画布在窗口里的位置。
fn moved(path: &Path<Pixels>, by: Point<Pixels>) -> Path<Pixels> {
    let mut path = path.clone();
    path.bounds.origin += by;
    for vertex in &mut path.vertices {
        vertex.xy_position += by;
    }
    path
}

/// 一帧要画的东西。
pub struct Scene {
    pub rings: Arc<Rings>,
    /// 悬停 / 选中的块在 `rings.slices` 里的下标。
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    /// 只突出这个类型，其余的淡出。
    pub filter: Option<Category>,
    pub unit: SizeUnit,
    pub palette: Palette,
    /// 被裁掉的小项目那一块上写的字。
    pub rest_label: SharedString,
    /// 圆心写的：没有悬停时是正在看的目录。
    pub title: SharedString,
    pub total: u64,
}

fn slice_color(slice: &Slice, dark: bool) -> Hsla {
    let base = if slice.kind == Kind::Rest {
        if dark {
            hsla(0.6, 0.06, 0.30, 1.0)
        } else {
            hsla(0.6, 0.06, 0.62, 1.0)
        }
    } else {
        category_color(slice.category, dark)
    };
    // 越往外越浅，一圈一圈的层次看得出来。
    let lift = f32::from(slice.depth - 1) * 0.045;
    hsla(base.h, base.s, (base.l + lift).min(0.78), 1.0)
}

/// 在画布 `bounds` 里画出整张旋转图。
pub fn paint(scene: &Scene, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let palette = scene.palette;
    let rings = &scene.rings;
    let origin = bounds.origin;
    window.paint_quad(fill(bounds, palette.surface));

    for slice in &rings.slices {
        window.paint_path(
            moved(&slice.outline, origin),
            slice_color(slice, palette.dark),
        );
        if let Some(category) = scene.filter {
            let fade = (1.0 - slice.share(category)) * 0.8;
            if fade > 0.02 {
                window.paint_path(moved(&slice.outline, origin), palette.surface.opacity(fade));
            }
        }
    }

    // 够大的弧上写名字和大小，放不下就不写。
    for slice in &rings.slices {
        let (room_w, room_h) = slice.room();
        if room_w < 56.0 || room_h < 16.0 || slice.outer - slice.inner < 16.0 {
            continue;
        }
        let name: &str = if slice.kind == Kind::Rest {
            &scene.rest_label
        } else {
            &slice.name
        };
        let (x, y) = slice.centroid(rings.center);
        let width = room_w - 8.0;
        let Some(line) = shape(
            window,
            name,
            11.0,
            hsla(0.0, 0.0, 1.0, 0.96),
            FontWeight::MEDIUM,
            width,
        ) else {
            continue;
        };
        let two_rows = room_h >= 34.0;
        let top = y - if two_rows { 14.0 } else { 7.0 };
        draw(
            window,
            cx,
            &line,
            bounds,
            x - line.width.as_f32() / 2.0,
            top,
        );
        if two_rows
            && let Some(line) = shape(
                window,
                &fmt::bytes(slice.size, scene.unit),
                10.5,
                hsla(0.0, 0.0, 1.0, 0.72),
                FontWeight::NORMAL,
                width,
            )
        {
            draw(
                window,
                cx,
                &line,
                bounds,
                x - line.width.as_f32() / 2.0,
                top + 14.0,
            );
        }
    }

    // 悬停的浅浅提亮；选中的描一圈边。
    if let Some(slice) = scene
        .hover
        .filter(|&index| Some(index) != scene.selected)
        .and_then(|index| rings.slices.get(index))
    {
        window.paint_path(moved(&slice.outline, origin), palette.text.opacity(0.22));
    }
    if let Some(slice) = scene.selected.and_then(|index| rings.slices.get(index)) {
        window.paint_path(moved(&slice.outline, origin), palette.text.opacity(0.14));
        let mut edge = PathBuilder::stroke(px(2.5));
        trace(
            &mut edge,
            (
                rings.center.0 + origin.x.as_f32(),
                rings.center.1 + origin.y.as_f32(),
            ),
            slice.inner + 1.2,
            slice.outer - 1.2,
            slice.from,
            slice.to,
        );
        if let Ok(path) = edge.build() {
            window.paint_path(path, palette.text);
        }
    }

    // 圆心：悬停的那一项，没有悬停就是正在看的目录。
    if rings.hole > 0.0 {
        let shown = scene.hover.and_then(|index| rings.slices.get(index));
        let (name, size): (&str, u64) = match shown {
            Some(slice) if slice.kind == Kind::Rest => (&scene.rest_label, slice.size),
            Some(slice) => (&slice.name, slice.size),
            None => (&scene.title, scene.total),
        };
        let width = rings.hole * 1.7;
        let name_line = shape(
            window,
            name,
            12.0,
            palette.text,
            FontWeight::SEMIBOLD,
            width,
        );
        let size_line = shape(
            window,
            &fmt::bytes(size, scene.unit),
            11.5,
            palette.muted,
            FontWeight::NORMAL,
            width,
        );
        let (cx0, cy0) = rings.center;
        let top = cy0
            - if name_line.is_some() && size_line.is_some() {
                14.0
            } else {
                7.0
            };
        if let Some(line) = name_line {
            draw(
                window,
                cx,
                &line,
                bounds,
                cx0 - line.width.as_f32() / 2.0,
                top,
            );
        }
        if let Some(line) = size_line {
            draw(
                window,
                cx,
                &line,
                bounds,
                cx0 - line.width.as_f32() / 2.0,
                top + 15.0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pc_specs_core::space::{Options, Progress, Tree, scan_with};
    use std::fs;
    use std::path::Path as FsPath;
    use std::sync::atomic::AtomicBool;

    fn write(path: &FsPath, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7u8; bytes]).unwrap();
    }

    fn sample_tree(tag: &str) -> Tree {
        let dir = std::env::temp_dir().join(format!("pcs-sunburst-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("a/a1.bin"), 200 * 1024);
        write(&dir.join("a/a2.bin"), 100 * 1024);
        write(&dir.join("b/sub/s1.mp4"), 900 * 1024);
        write(&dir.join("b/b1.txt"), 300 * 1024);
        write(&dir.join("c.bin"), 400 * 1024);
        let options = Options {
            base_keep: 0,
            ..Options::default()
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
    fn rings_nest_without_overlap_and_in_proportion() {
        let tree = sample_tree("nest");
        let rings = layout(&tree.root, &[], 600.0, 500.0, 3);
        assert!(!rings.slices.is_empty());

        let first: Vec<&Slice> = rings.slices.iter().filter(|s| s.path.len() == 1).collect();
        // 第一圈把整个圆铺满，弧长和大小成正比。
        let covered: f32 = first.iter().map(|s| s.to - s.from).sum();
        assert!((covered - TAU).abs() < 1e-3, "{covered}");
        let total: u64 = first.iter().map(|s| s.size).sum();
        for slice in &first {
            let expect = TAU * slice.size as f32 / total as f32;
            assert!((slice.to - slice.from - expect).abs() < 1e-3);
        }
        // 子项的弧躺在父项的弧里面、紧挨着往外一圈。
        for slice in &rings.slices {
            if slice.path.len() > 1 {
                let parent = &slice.path[..slice.path.len() - 1];
                let parent =
                    &rings.slices[rings.find(parent).expect("a nested slice has its parent")];
                assert!(slice.from >= parent.from - 1e-4 && slice.to <= parent.to + 1e-4);
                assert!((slice.inner - parent.outer).abs() < 1e-3);
            }
        }
        assert!(
            rings.slices.iter().any(|s| s.path.len() == 3),
            "b/sub/* is drawn"
        );
    }

    /// 排好的形状必须真的画在自己的弧里：弧的方向（sweep）弄反，就会绕大半圈画到别处去。
    #[test]
    fn outlines_stay_inside_their_slice() {
        let tree = sample_tree("outline");
        let rings = layout(&tree.root, &[], 600.0, 500.0, 3);
        let (cx, cy) = rings.center;
        for slice in &rings.slices {
            assert!(!slice.outline.vertices.is_empty(), "{:?}", slice.path);
            let middle = (slice.from + slice.to) / 2.0;
            for vertex in &slice.outline.vertices {
                let (dx, dy) = (
                    vertex.xy_position.x.as_f32() - cx,
                    vertex.xy_position.y.as_f32() - cy,
                );
                let radius = dx.hypot(dy);
                assert!(
                    radius >= slice.inner - 0.5 && radius <= slice.outer + 0.5,
                    "{radius} outside {}..{} for {:?}",
                    slice.inner,
                    slice.outer,
                    slice.path
                );
                // 角度换算到以弧的中线为中心的 (-π, π]，免得在 0 / 2π 附近绕回去。
                let mut angle = dx.atan2(-dy) - middle;
                while angle > PI {
                    angle -= TAU;
                }
                while angle <= -PI {
                    angle += TAU;
                }
                let half = (slice.to - slice.from) / 2.0;
                assert!(
                    angle.abs() <= half + 0.01,
                    "{angle} beyond ±{half} for {:?}",
                    slice.path
                );
            }
        }
    }

    #[test]
    fn levels_limit_how_many_rings_are_drawn() {
        let tree = sample_tree("levels");
        let one = layout(&tree.root, &[], 600.0, 500.0, 1);
        assert!(one.slices.iter().all(|s| s.path.len() == 1));
        let many = layout(&tree.root, &[], 600.0, 500.0, 4);
        assert!(many.slices.len() > one.slices.len());
    }

    #[test]
    fn hit_finds_the_slice_under_a_point() {
        let tree = sample_tree("hit");
        let rings = layout(&tree.root, &[], 600.0, 500.0, 3);
        for (index, slice) in rings.slices.iter().enumerate() {
            let (x, y) = slice.centroid(rings.center);
            assert_eq!(rings.hit(x, y), Some(index), "{:?}", slice.path);
            assert_eq!(rings.find(&slice.path), Some(index));
        }
        // 圆心的空洞不属于任何一块，却是「回到上一级」的点击区。
        let (cx, cy) = rings.center;
        assert_eq!(rings.hit(cx, cy), None);
        assert!(rings.in_hole(cx, cy));
        assert!(!rings.in_hole(cx + rings.hole + 5.0, cy));
        // 圆环外面什么也没有。
        assert_eq!(rings.hit(0.0, 0.0), None);
        assert_eq!(rings.find(&[99]), None);
    }

    #[test]
    fn a_tiny_canvas_or_missing_view_draws_nothing() {
        let tree = sample_tree("tiny");
        assert!(layout(&tree.root, &[], 60.0, 60.0, 3).slices.is_empty());
        assert!(
            layout(&tree.root, &[99, 1], 600.0, 500.0, 3)
                .slices
                .is_empty()
        );
        assert!(layout(&tree.root, &[], 600.0, 500.0, 0).slices.is_empty());
    }

    #[test]
    fn a_single_item_fills_the_whole_ring() {
        let dir = std::env::temp_dir().join(format!("pcs-sunburst-solo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("only.bin"), 500 * 1024);
        let options = Options {
            base_keep: 0,
            ..Options::default()
        };
        let tree = scan_with(
            &dir,
            &options,
            &Progress::default(),
            &AtomicBool::new(false),
        )
        .expect("scan");
        let _ = fs::remove_dir_all(&dir);
        let rings = layout(&tree.root, &[], 400.0, 400.0, 2);
        assert_eq!(rings.slices.len(), 1);
        let slice = &rings.slices[0];
        assert!((slice.to - slice.from - TAU).abs() < 1e-3);
        // 整圈的弧也能被点中，也能画出路径。
        let (x, y) = (
            rings.center.0,
            rings.center.1 - (slice.inner + slice.outer) / 2.0,
        );
        assert_eq!(rings.hit(x, y), Some(0));
        assert!(!slice.outline.vertices.is_empty());
    }
}
