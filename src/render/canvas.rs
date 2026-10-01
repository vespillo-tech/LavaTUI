//! What a style draws from, and the cell loops every style shares.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::cell::half_block;
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
        for cy in 0..usize::from(self.area.height) {
            for cx in 0..usize::from(self.area.width) {
                let at = self.cell_at(cx, cy);
                f(at, self.cell_mut(buf, &at));
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

    /// Draw a half-block canvas (1×2 pixels per cell): `pixel(x, y)` is
    /// `None` for an empty pixel (the liquid shows) or ink of a colour.
    #[inline]
    pub fn draw_half_blocks(
        &self,
        buf: &mut Buffer,
        mut pixel: impl FnMut(usize, usize) -> Option<Color>,
    ) {
        self.for_each_cell(buf, |at, cell| {
            let (top, bottom) = (pixel(at.x, at.y), pixel(at.x, at.y + 1));
            half_block(cell, top, bottom, at.base);
        });
    }
}
