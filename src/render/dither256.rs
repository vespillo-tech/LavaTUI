//! 256 colours: resolve the blends a [`Theme::dithering`] theme drew as RGB
//! into xterm indices, ordered-dithered pixel by pixel.
//!
//! The pattern is the 8×8 Bayer matrix of [`bayer`], anchored to the lamp,
//! so a colour that holds still keeps the same cells (bandwidth, §7), and a
//! style that already dithers its own bands on that matrix (heatmap) lines
//! up with it instead of beating against it. Half-block cells are dithered
//! per pixel: a cell drawn whole (`█`, or a blank) whose two pixels resolve
//! differently becomes `▀`. Other glyphs take one threshold per cell.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::bayer;
use crate::theme::Theme;

pub fn resolve(theme: &Theme, area: Rect, buf: &mut Buffer) {
    for cy in 0..area.height {
        for cx in 0..area.width {
            let cell = &mut buf[(area.x + cx, area.y + cy)];
            let (x, y) = (usize::from(cx), usize::from(cy));
            let pixel = |c: Color, row: usize| theme.dither(c, bayer(x, 2 * y + row));
            let (fg, bg) = (cell.fg, cell.bg);
            match cell.symbol() {
                "▀" => {
                    cell.fg = pixel(fg, 0);
                    cell.bg = pixel(bg, 1);
                }
                "▄" => {
                    cell.fg = pixel(fg, 1);
                    cell.bg = pixel(bg, 0);
                }
                whole @ ("█" | " ") => {
                    let c = if whole == " " { bg } else { fg };
                    let (top, bottom) = (pixel(c, 0), pixel(c, 1));
                    if top == bottom {
                        if whole == " " {
                            cell.bg = top;
                        } else {
                            cell.fg = top;
                        }
                    } else {
                        cell.set_char('▀').set_fg(top).set_bg(bottom);
                    }
                }
                _ => {
                    // Offset the background's threshold, so a glyph and
                    // its backdrop in near colours don't flip together.
                    cell.fg = theme.dither(fg, bayer(x, y));
                    cell.bg = theme.dither(bg, bayer(x + 4, y + 4));
                }
            }
        }
    }
}
