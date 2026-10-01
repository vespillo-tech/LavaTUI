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
    pub(super) light: Option<&'a [f32]>,
    /// Per sample row: the container's `[lo, hi)` sample columns.
    pub(super) mask: &'a [(usize, usize)],
    pub width: usize,
    pub height: usize,
    pub theme: &'a Theme,
    /// Seconds since launch, for styles that animate on their own.
    pub time: f64,
}

/// One terminal cell of the canvas, as handed to a style's cell loop.
#[derive(Debug, Clone, Copy)]
pub struct At {
    /// Cell column and row within the lamp's area.
    pub cx: usize,
    pub cy: usize,
    /// The cell's top-left sample pixel.
    pub x: usize,
    pub y: usize,
    /// What shows there with no wax ([`Canvas::backdrop`] of the top-left
    /// pixel), and its colour.
    pub backdrop: Ink,
    pub base: Color,
}

impl Canvas<'_> {
    /// Sample at pixel (`x`, `y`); `y` runs down.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> Sample {
        self.samples[y * self.width + x]
    }

    /// Brightness factor at a pixel: 1.0 unless a lighting pass ran.
    #[inline]
    pub fn light(&self, x: usize, y: usize) -> f32 {
        self.light.map_or(1.0, |l| l[y * self.width + x])
    }

    /// Whether the pixel is inside the container (always, in bleed).
    #[inline]
    pub fn inside(&self, x: usize, y: usize) -> bool {
        let (lo, hi) = self.mask[y];
        (lo..hi).contains(&x)
    }

    /// What shows where there's no wax: `liquid` inside the container,
    /// `bg` outside it.
    #[inline]
    pub fn backdrop(&self, x: usize, y: usize) -> Ink {
        Ink::Role(if self.inside(x, y) {
            Role::Liquid
        } else {
            Role::Bg
        })
    }

    /// Call `f` once for every cell of the area, row by row. The canvas
    /// covers the area exactly, so the grid is its size over the area's.
    #[inline]
    pub fn for_each_cell(&self, buf: &mut Buffer, mut f: impl FnMut(At, &mut Cell)) {
        let area = self.area;
        if area.is_empty() {
            return;
        }
        let gx = self.width / usize::from(area.width);
        let gy = self.height / usize::from(area.height);
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (x, y) = (gx * cx, gy * cy);
                let backdrop = self.backdrop(x, y);
                let at = At {
                    cx,
                    cy,
                    x,
                    y,
                    backdrop,
                    base: self.theme.color(backdrop),
                };
                f(at, &mut buf[(area.x + cx as u16, area.y + cy as u16)]);
            }
        }
    }

    /// Draw a half-block canvas (1×2 pixels per cell): `pixel(x, y)` is
    /// `None` for an empty pixel (the backdrop shows) or ink of a colour.
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
