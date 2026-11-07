//! 自适应排版：把一串有顺序的磁贴铺满任意形状的屏幕。
//!
//! 做法是「有序切分」：把整块区域沿横向或纵向一刀切成两半，面积按两边磁贴的权重（宽×高）分配，
//! 每一半再递归地切，直到每块只剩一个磁贴。磁贴始终保持用户设定的先后顺序，面积严格正比于大小，
//! 没有空洞。
//!
//! 每一刀切在哪里、横切还是纵切，由代价函数决定：磁贴的宽高比越接近理想值越好。
//! 所以同样 9 个磁贴，横屏会排成 3×3，竖屏会变成 2 列 5 行，超宽屏会拉成一长排。
//! 整个过程只依赖区域的宽高比，所以放大缩小屏幕不会改变排法，预览图和真实大屏一致。

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 一个磁贴的相对大小（1–3）。
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub w: u8,
    pub h: u8,
}

/// 单位格（宽 1 高 1）的设计尺寸，组件里的字号、间距都是按它设计的。
const UNIT_WIDTH: f32 = 360.;
const UNIT_HEIGHT: f32 = 220.;

/// 磁贴至少要有这么大（设计像素）才摆得下一个读数加一张小图。
const MIN_HEIGHT: f32 = 110.;
const MIN_WIDTH: f32 = 170.;

/// 单位格理想的宽高比。
const IDEAL_ASPECT: f32 = 1.6;

/// 排布的结果。
pub struct Arrangement {
    /// 保留下来的磁贴的矩形，顺序与传入的一致；比传入的少，说明屏幕太小，末尾的被舍弃了。
    pub rects: Vec<Rect>,
    /// 一个设计像素对应多少屏幕像素：单位格的缩放，已经乘上了用户设置的缩放。
    pub unit: f32,
}

impl Arrangement {
    /// 某个磁贴里内容的缩放。同一块屏幕上的磁贴共用 `unit`，所以字号一致，大磁贴只是内容区更大、
    /// 摆得下更多东西；只有细长的磁贴才会被压低，免得内容区小到什么都放不下。
    pub fn scale_of(&self, rect: &Rect) -> f32 {
        self.unit.min(rect.h / MIN_HEIGHT).min(rect.w / MIN_WIDTH)
    }
}

/// 单位格的缩放：1 表示一个单位格刚好是设计尺寸（360×220 像素）。
fn cell_scale(cells: &[Cell], width: f32, height: f32) -> f32 {
    let weight: f32 = cells
        .iter()
        .map(|cell| f32::from(cell.w) * f32::from(cell.h))
        .sum();
    if weight <= 0. {
        return 1.;
    }
    (width * height / weight / (UNIT_WIDTH * UNIT_HEIGHT)).sqrt()
}

/// 在 `width × height` 的区域里排布 `cells`，返回每个磁贴的矩形（原点在区域左上角）。
pub fn arrange(cells: &[Cell], width: f32, height: f32, gap: f32) -> Vec<Rect> {
    if cells.is_empty() || width <= 0. || height <= 0. {
        return Vec::new();
    }
    let mut solver = Solver::new(cells);
    let mut rects = vec![Rect::default(); cells.len()];
    let whole = Rect {
        x: 0.,
        y: 0.,
        w: width,
        h: height,
    };
    solver.place(0, cells.len(), whole, gap.max(0.), &mut rects);
    rects
}

/// 排布并算出缩放。`boost` 是用户设置的缩放（1 = 100%）。
/// 屏幕太小、放不下所有磁贴时，从末尾开始舍弃，直到每个磁贴里的字不小于 `min_scale`（或只剩一个）。
pub fn fit(
    cells: &[Cell],
    width: f32,
    height: f32,
    gap: f32,
    boost: f32,
    min_scale: f32,
) -> Arrangement {
    let mut count = cells.len();
    loop {
        let kept = &cells[..count];
        let arrangement = Arrangement {
            rects: arrange(kept, width, height, gap),
            unit: cell_scale(kept, width, height) * boost,
        };
        let readable = arrangement
            .rects
            .iter()
            .all(|rect| arrangement.scale_of(rect) >= min_scale);
        if count <= 1 || readable {
            return arrangement;
        }
        count -= 1;
    }
}

#[derive(Clone, Copy)]
enum Split {
    Leaf,
    /// 在第 n 个磁贴前左右切开。
    SideBySide(usize),
    /// 在第 n 个磁贴前上下切开。
    Stacked(usize),
}

struct Solver {
    /// 每个磁贴的面积权重（宽×高）。
    weight: Vec<f32>,
    /// 每个磁贴理想的宽高比。
    target: Vec<f32>,
    /// 权重的前缀和：区间 [a, b) 的总权重是 `prefix[b] - prefix[a]`。
    prefix: Vec<f32>,
    /// （起点，终点，宽高比的对数分档）→ 最小代价和对应的切法。
    memo: HashMap<(usize, usize, i32), (f32, Split)>,
}

impl Solver {
    fn new(cells: &[Cell]) -> Self {
        let weight: Vec<f32> = cells
            .iter()
            .map(|cell| f32::from(cell.w) * f32::from(cell.h))
            .collect();
        let target = cells
            .iter()
            .map(|cell| IDEAL_ASPECT * f32::from(cell.w) / f32::from(cell.h))
            .collect();
        let mut prefix = vec![0.];
        for value in &weight {
            prefix.push(prefix.last().copied().unwrap_or(0.) + value);
        }
        Self {
            weight,
            target,
            prefix,
            memo: HashMap::new(),
        }
    }

    /// 把 [from, to) 这段磁贴铺进宽高比为 `aspect` 的矩形，最小的代价是多少、第一刀怎么切。
    /// 代价是各磁贴宽高比偏离理想值的程度（取对数，太宽和太窄一视同仁），按面积加权。
    fn solve(&mut self, from: usize, to: usize, aspect: f32) -> (f32, Split) {
        let aspect = aspect.clamp(0.02, 50.);
        if to - from == 1 {
            let off = (aspect / self.target[from]).ln();
            return (off * off * self.weight[from], Split::Leaf);
        }
        let key = (from, to, (aspect.ln() * 25.).round() as i32);
        if let Some(found) = self.memo.get(&key) {
            return *found;
        }
        let total = self.prefix[to] - self.prefix[from];
        let mut best = (f32::INFINITY, Split::Leaf);
        for at in from + 1..to {
            let share = (self.prefix[at] - self.prefix[from]) / total;
            let side = self.solve(from, at, aspect * share).0
                + self.solve(at, to, aspect * (1. - share)).0;
            if side < best.0 {
                best = (side, Split::SideBySide(at));
            }
            let stacked = self.solve(from, at, aspect / share).0
                + self.solve(at, to, aspect / (1. - share)).0;
            if stacked < best.0 {
                best = (stacked, Split::Stacked(at));
            }
        }
        self.memo.insert(key, best);
        best
    }

    fn place(&mut self, from: usize, to: usize, area: Rect, gap: f32, out: &mut [Rect]) {
        if to - from == 1 {
            out[from] = area;
            return;
        }
        let (_, split) = self.solve(from, to, area.w / area.h.max(1.));
        let (at, side_by_side) = match split {
            Split::SideBySide(at) => (at, true),
            Split::Stacked(at) => (at, false),
            Split::Leaf => {
                out[from..to].fill(area);
                return;
            }
        };
        let share = (self.prefix[at] - self.prefix[from]) / (self.prefix[to] - self.prefix[from]);
        if side_by_side {
            let usable = (area.w - gap).max(0.);
            let first = usable * share;
            self.place(from, at, Rect { w: first, ..area }, gap, out);
            let rest = Rect {
                x: area.x + first + gap,
                w: usable - first,
                ..area
            };
            self.place(at, to, rest, gap, out);
        } else {
            let usable = (area.h - gap).max(0.);
            let first = usable * share;
            self.place(from, at, Rect { h: first, ..area }, gap, out);
            let rest = Rect {
                y: area.y + first + gap,
                h: usable - first,
                ..area
            };
            self.place(at, to, rest, gap, out);
        }
    }
}
