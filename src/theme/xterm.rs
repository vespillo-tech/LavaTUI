//! Nearest xterm-256 colour, in O(1): the 6×6×6 cube is separable per
//! channel, so pick the nearest level on each axis, then compare with the
//! nearest step of the 24-step grey ramp.

use super::Rgb;

/// Channel levels of the colour cube (indices 16..=231).
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

pub fn nearest(c: Rgb) -> u8 {
    let level = |v: u8| {
        // Midpoints between cube levels.
        match v {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    };
    let (r, g, b) = (level(c.0), level(c.1), level(c.2));
    let cube = Rgb(CUBE[r], CUBE[g], CUBE[b]);
    let cube_index = 16 + 36 * r as u8 + 6 * g as u8 + b as u8;

    // Greys 232..=255 are 8, 18, …, 238.
    let mean = (u16::from(c.0) + u16::from(c.1) + u16::from(c.2)) / 3;
    let step = (mean.saturating_sub(3) / 10).min(23) as u8;
    let v = 8 + 10 * step;
    let grey = Rgb(v, v, v);

    if dist(c, grey) < dist(c, cube) {
        232 + step
    } else {
        cube_index
    }
}

/// Perceptually weighted squared distance (green counts most).
fn dist(a: Rgb, b: Rgb) -> u32 {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2) as u32;
    2 * d(a.0, b.0) + 4 * d(a.1, b.1) + 3 * d(a.2, b.2)
}
