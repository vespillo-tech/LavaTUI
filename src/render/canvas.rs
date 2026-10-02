//! What a style draws from, and the cell loops every style shares.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::cell::{HalfBlocks, Pixel};
use crate::sim::Sample;
use crate::theme::{Ink, Role, Theme};

/// The sampled field at a style's grid, plus everything it needs to colour
/// it.
pub struct Canvas<'a> {
    /// The cells being drawn.
    pub area: Rect,
    pub(super) samples: &'a [Sample],
    pub width: usize,
    pub height: usize,
    pub theme: &'a Theme,
    /// Seconds since launch, for styles that animate on their own.
    pub time: f64,
    /// The terminal shows cell backgrounds see-through (glyphs opaque):
    /// see [`super::cell::half_block`].
    pub translucent: bool,
}

/// What shows where there's no wax.
pub const LIQUID: Ink = Ink::Role(Role::Liquid);

/// One terminal cell of the canvas, as handed to a style's cell loop.
#[derive(Debug, Clone, Copy)]
pub struct At {
    /// Cell column and row within the lamp's area.
    pub cx: usize,
    pub cy: usize,
    /// The cell's top-left sample pixel.
    pub x: usize,
    pub y: usize,
    /// What shows there with no wax: the liquid's colour.
    pub base: Color,
}

impl Canvas<'_> {
    /// Sample at pixel (`x`, `y`); `y` runs down.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> Sample {
        self.samples[y * self.width + x]
    }

    /// Call `f` once for every cell of the area, row by row.
    #[inline]
    pub fn for_each_cell(&self, buf: &mut Buffer, mut f: impl FnMut(At, &mut Cell)) {
        let width = usize::from(self.area.width);
        if width == 0 || self.area.height == 0 {
            return;
        }
        let gx = self.width / width;
        let gy = self.height / usize::from(self.area.height);
        let base = self.theme.color(LIQUID);
        for cy in 0..usize::from(self.area.height) {
            // Validate/index once per row, rather than through Buffer's
            // coordinate lookup for every cell. The lamp may be offset
            // within a wider buffer, so keep the buffer's row stride.
            let start = buf.index_of(self.area.x, self.area.y + cy as u16);
            for (cx, cell) in buf.content[start..start + width].iter_mut().enumerate() {
                let at = At {
                    cx,
                    cy,
                    x: gx * cx,
                    y: gy * cy,
                    base,
                };
                f(at, cell);
            }
        }
    }

    /// Cell (`cx`, `cy`) of the area. The canvas covers the area exactly,
    /// so the grid is its size over the area's.
    #[inline]
    pub fn cell_at(&self, cx: usize, cy: usize) -> At {
        let gx = self.width / usize::from(self.area.width);
        let gy = self.height / usize::from(self.area.height);
        At {
            cx,
            cy,
            x: gx * cx,
            y: gy * cy,
            base: self.theme.color(LIQUID),
        }
    }

    /// The buffer cell `at` draws into.
    #[inline]
    pub fn cell_mut<'b>(&self, buf: &'b mut Buffer, at: &At) -> &'b mut Cell {
        &mut buf[(self.area.x + at.cx as u16, self.area.y + at.cy as u16)]
    }

    /// Draw a half-block canvas (1×2 pixels per cell) of `pixel(x, y)`s.
    #[inline]
    pub fn draw_half_blocks(&self, buf: &mut Buffer, mut pixel: impl FnMut(usize, usize) -> Pixel) {
        let mut cells = HalfBlocks::new(self.theme, self.theme.color(LIQUID), self.translucent);
        self.for_each_cell(buf, |at, cell| {
            cells.draw(cell, pixel(at.x, at.y), pixel(at.x, at.y + 1));
        });
    }
}
