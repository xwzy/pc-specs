//! 监控面板里「一格一格」的 LED 式图表。直接用 canvas 画方块，不依赖图表库。
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, Bounds, FontFeatures, Hsla, IntoElement, Styled as _, canvas, fill, point, px, size,
};
use std::sync::Arc;

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
                    let y = bounds.origin.y
                        + px(height - (row + 1) as f32 * cell_h - row as f32 * gap.1);
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

/// 等宽数字：数值每秒跳动时不会因为字宽不同而左右抖。
pub fn tabular<E: gpui_kit::Styled>(element: E) -> E {
    element.font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
}
