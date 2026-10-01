//! The glass lamp's silhouette (docs/design.md §2.1), defined once. The
//! sim's bottle walls, the layout's cap / bottle / base rows, the glass's
//! metal and the render mask all come from here, so they always meet.
//!
//! ```text
//!     ▄██▄        cap   0.18 → 0.40 of the lamp width   (15 % of the rows)
//!    ╱    ╲
//!   │      │      bottle 0.40 neck, 0.78 bulge 28 % up, 0.56 foot
//!    ╲    ╱
//!   ▟██████▙      base  0.56 → 1.0                       (22 % of the rows)
//! ```
//!
//! Widths here are fractions of the lamp's full width unless they say
//! otherwise. Pure geometry: no terminal, no state.

use crate::sim::Shape;

/// Lamp width ÷ height in cells, at a cell aspect of 2.0 (§1.4:
/// `W = 0.8 Ht`).
pub const LAMP_WIDTH: f64 = 0.8;

/// The bottle's narrowest point, at the top where the cap sits.
pub const NECK: f64 = 0.40;
/// The bottle's widest point, and how far up the bottle it sits.
pub const BULGE: f64 = 0.78;
pub const BULGE_Y: f64 = 0.28;
/// The bottle's width at its foot, where the base takes over.
pub const FOOT: f64 = 0.56;

/// The cap's width from its top down to the neck, and the base's from the
/// foot down to the full width.
pub const CAP: (f64, f64) = (0.18, NECK);
pub const BASE: (f64, f64) = (FOOT, 1.0);

/// The cap's and base's share of the lamp's rows, each at least
/// [`MIN_PART_ROWS`].
pub const CAP_ROWS: f64 = 0.15;
pub const BASE_ROWS: f64 = 0.22;
pub const MIN_PART_ROWS: u16 = 2;

/// Columns either side of the bottle (the lamp view), as a share of the
/// lamp's width: `(1 - BULGE) / 2`, written out because the layout rounds
/// it and its snapshots pin that rounding.
pub const BOTTLE_INSET: f64 = 0.11;

/// The bottle world's fixed aspect (bounding box width ÷ height): this
/// silhouette at the default cell aspect. Glass never resizes the sim.
pub const BOTTLE_ASPECT: f64 = 0.5;

/// Bottle width at height `y` (0 foot … 1 neck), as a fraction of its
/// widest (the bulge): straight lines foot → bulge → neck.
pub fn bottle_width(y: f64) -> f64 {
    let (foot, neck) = (FOOT / BULGE, NECK / BULGE);
    let y = y.clamp(0.0, 1.0);
    if y < BULGE_Y {
        foot + (1.0 - foot) * y / BULGE_Y
    } else {
        1.0 + (neck - 1.0) * (y - BULGE_Y) / (1.0 - BULGE_Y)
    }
}

/// The bottle's area as a share of its bounding box (two trapezoids).
pub fn bottle_area() -> f64 {
    let f = bottle_width;
    (BULGE_Y * (f(0.0) + f(BULGE_Y)) + (1.0 - BULGE_Y) * (f(BULGE_Y) + f(1.0))) / 2.0
}

/// Cap, bottle and base rows of a glass lamp `rows` tall.
pub fn part_rows(rows: u16) -> (u16, u16, u16) {
    let part = |share: f64| ((f64::from(rows) * share).round() as u16).max(MIN_PART_ROWS);
    let (cap, base) = (part(CAP_ROWS), part(BASE_ROWS));
    (cap, rows - cap - base, base)
}

/// Half-width of `shape` at height `world_y` (0 bottom … 1 top) in a view
/// `cols` wide, in half columns either side of the view's centre. The same
/// rounding as the glass cap and base, so they meet flush.
pub fn wall(shape: Shape, cols: u16, world_y: f64) -> u32 {
    let n = (shape.width_fraction(world_y) * f64::from(cols)).round();
    n.clamp(1.0, f64::from(cols)) as u32
}

/// The inside span `[lo, hi)` of `shape` at height `world_y` in a view
/// `cols` wide, in half columns from the view's left edge.
pub fn row_span(shape: Shape, cols: u16, world_y: f64) -> (u32, u32) {
    let n = wall(shape, cols, world_y);
    (u32::from(cols) - n, u32::from(cols) + n)
}

/// Height (0 bottom … 1 top) of the middle of row `row` of `rows`,
/// counting down from the top.
pub fn row_height(row: u32, rows: u32) -> f64 {
    1.0 - (f64::from(row) + 0.5) / f64::from(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_meets_the_cap_and_base() {
        assert!((bottle_width(0.0) * BULGE - BASE.0).abs() < 1e-15);
        assert_eq!(bottle_width(BULGE_Y), 1.0);
        assert!((bottle_width(1.0) * BULGE - CAP.1).abs() < 1e-15);
        assert!((BOTTLE_INSET - (1.0 - BULGE) / 2.0).abs() < 1e-15);
    }

    #[test]
    fn parts_fill_the_lamp() {
        for rows in 6..200 {
            let (cap, bottle, base) = part_rows(rows);
            assert_eq!(cap + bottle + base, rows);
            assert!(cap >= MIN_PART_ROWS && base >= MIN_PART_ROWS);
        }
    }
}
