//! 监控面板里的图形部件：一格一格的 LED 走势、条形，和环形仪表。
//! 直接用 canvas 画方块和圆弧，不依赖图表库的布局。
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::plot::shape::{Arc, ArcData};
use gpui_kit::{
    App, Bounds, FontFeatures, Hsla, IntoElement, Styled as _, canvas, fill, point, px, size,
};
use std::f32::consts::TAU;
use std::sync::Arc as Shared;

/// 点亮的格子怎么上色：低处用主色，越过阈值的格子换成警告 / 危险色，像音量表一样。
#[derive(Clone, Copy)]
pub struct Palette {
    lit: Hsla,
    warn: Hsla,
    danger: Hsla,
    off: Hsla,
    warn_at: f32,
    danger_at: f32,
}

impl Palette {
    pub fn new(lit: Hsla, cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            lit,
            warn: theme.warning,
            danger: theme.danger,
            off: theme.foreground.opacity(0.08),
            warn_at: 0.75,
            danger_at: 0.9,
        }
    }

    /// 不分级：整条都是主色（网速、读写这类没有「危险」概念的数据）。
    pub fn plain(mut self) -> Self {
        self.warn_at = 2.;
        self.danger_at = 2.;
        self
    }

    /// 自定义分级位置（0..=1，相对满量程）。
    pub fn levels(mut self, warn_at: f32, danger_at: f32) -> Self {
        self.warn_at = warn_at;
        self.danger_at = danger_at;
        self
    }

    /// `top` 是这一格上沿相对满量程的位置。
    fn cell(&self, top: f32) -> Hsla {
        if top > self.danger_at + 1e-4 {
            self.danger
        } else if top > self.warn_at + 1e-4 {
            self.warn
        } else {
            self.lit
        }
    }
}

/// 点亮几格：有读数时至少亮一格，免得小于一格的读数看起来像断了。
fn lit_cells(value: f32, rows: usize) -> usize {
    if !value.is_finite() || value <= 0.005 {
        0
    } else {
        ((value.min(1.) * rows as f32).ceil() as usize).clamp(1, rows)
    }
}

/// 竖向的一列列格子。`values` 取 0..=1，最新的在最右；不够 `cols` 个时左边留空。
/// 历史曲线（很多列、间距小）和逐核心条形（几列、间距大）都用它。
pub fn columns(
    values: Vec<Option<f32>>,
    cols: usize,
    rows: usize,
    gap: (f32, f32),
    palette: Palette,
) -> impl IntoElement {
    grid(values, cols, rows, gap, palette, false)
}

/// 和 `columns` 一样，但格子从上往下长。和 `columns` 上下摆在一起就是一张对称的图：
/// 上行在中线之上、下行在中线之下（上传 / 下载、读 / 写）。
pub fn columns_down(
    values: Vec<Option<f32>>,
    cols: usize,
    rows: usize,
    gap: (f32, f32),
    palette: Palette,
) -> impl IntoElement {
    grid(values, cols, rows, gap, palette, true)
}

fn grid(
    values: Vec<Option<f32>>,
    cols: usize,
    rows: usize,
    gap: (f32, f32),
    palette: Palette,
    hanging: bool,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (width, height) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            let cell_w = (width - gap.0 * (cols.saturating_sub(1)) as f32) / cols as f32;
            let cell_h = (height - gap.1 * (rows.saturating_sub(1)) as f32) / rows as f32;
            let offset = cols.saturating_sub(values.len());
            for col in 0..cols {
                let value = col
                    .checked_sub(offset)
                    .and_then(|i| values.get(i).copied().flatten());
                let lit = value.map_or(0, |v| lit_cells(v, rows));
                for row in 0..rows {
                    let color = if row < lit {
                        palette.cell((row + 1) as f32 / rows as f32)
                    } else {
                        palette.off
                    };
                    let x = bounds.origin.x + px(col as f32 * (cell_w + gap.0));
                    let step = row as f32 * (cell_h + gap.1);
                    let y = bounds.origin.y
                        + px(if hanging {
                            step
                        } else {
                            height - cell_h - step
                        });
                    window.paint_quad(
                        fill(
                            Bounds::new(point(x, y), size(px(cell_w), px(cell_h))),
                            color,
                        )
                        .corner_radii(px(1.5_f32.min(cell_w / 2.))),
                    );
                }
            }
        },
    )
    .size_full()
}

/// 横向的一条格子，`fraction` 取 0..=1，从左往右点亮。
pub fn meter(fraction: f32, cells: usize, gap: f32, palette: Palette) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (width, height) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            let cell_w = (width - gap * (cells.saturating_sub(1)) as f32) / cells as f32;
            let lit = lit_cells(fraction, cells);
            for cell in 0..cells {
                let color = if cell < lit {
                    palette.cell((cell + 1) as f32 / cells as f32)
                } else {
                    palette.off
                };
                let x = bounds.origin.x + px(cell as f32 * (cell_w + gap));
                window.paint_quad(
                    fill(
                        Bounds::new(point(x, bounds.origin.y), size(px(cell_w), px(height))),
                        color,
                    )
                    .corner_radii(px(1.5_f32.min(cell_w / 2.).min(height / 2.))),
                );
            }
        },
    )
    .size_full()
}

/// 环形仪表：一圈暗色的底轨，上面从 12 点钟方向顺时针画出 `fraction`（0..=1）那一段，两头是圆的。
/// 大小固定为 `diameter` 见方，数字之类放在里面的内容由调用方叠上去。
pub fn ring(
    fraction: f32,
    diameter: f32,
    thickness: f32,
    tone: Hsla,
    track: Hsla,
) -> impl IntoElement {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0., 1.)
    } else {
        0.
    };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let outer = diameter / 2.;
            let inner = (outer - thickness).max(0.);
            let arc = Arc::new().inner_radius(inner).outer_radius(outer);
            arc.paint(&ArcData::new(&(), 0, 1., 0., TAU), track, &bounds, window);
            if fraction <= 0. {
                return;
            }
            let end = TAU * fraction;
            arc.paint(
                &ArcData::new(&(), 1, fraction, 0., end),
                tone,
                &bounds,
                window,
            );
            if fraction >= 0.999 {
                return;
            }
            // 圆头：在弧线的两端各补一个直径等于环宽的圆。
            let middle = (outer + inner) / 2.;
            let centre = point(
                bounds.origin.x.as_f32() + bounds.size.width.as_f32() / 2.,
                bounds.origin.y.as_f32() + bounds.size.height.as_f32() / 2.,
            );
            for angle in [0., end] {
                let x = centre.x + middle * angle.sin();
                let y = centre.y - middle * angle.cos();
                let half = thickness / 2.;
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(px(x - half), px(y - half)),
                            size(px(thickness), px(thickness)),
                        ),
                        tone,
                    )
                    .corner_radii(px(half)),
                );
            }
        },
    )
    .size(px(diameter))
}

/// 等宽数字：数值每秒跳动时不会因为字宽不同而左右抖。
pub fn tabular<E: gpui_kit::Styled>(element: E) -> E {
    element.font_features(FontFeatures(Shared::new(vec![("tnum".into(), 1)])))
}
