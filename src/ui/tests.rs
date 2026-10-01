//! Layout tests (docs/design.md §1.1): an exhaustive size sweep checking
//! the invariants, the mockup sizes checked element by element, and text
//! snapshots of the mockup layouts (`UPDATE_SNAPSHOTS=1 cargo test` to
//! rewrite `ui/snapshots/`, then review the diff).

use ratatui::layout::Rect;

use super::chrome::{HINTS, fit_hints, fit_words};
use super::layout::*;
use crate::clock::{self, Face, Tier};
use crate::dock::{Anchor, Place, clock_forms, clock_parts, pomodoro_forms};

/// A dock setup to lay out: where the clock (index 0) and the pomodoro
/// (index 1) go, the face, and which chips they'd show.
#[derive(Clone, Copy)]
struct Case {
    minimal: bool,
    status_bar: bool,
    clock: Place,
    pomodoro: Place,
    face: &'static dyn Face,
    hour24: bool,
    /// The clock's chip (none in minimal with `minimal.clock = "off"`).
    clock_chip: bool,
    /// A running pomodoro's chip width; `None` while idle.
    pomodoro_chip: Option<u16>,
    anchor: Anchor,
    cell_aspect: f64,
}

const CLOCK: usize = 0;
const POMODORO: usize = 1;

fn case(face: &'static dyn Face) -> Case {
    Case {
        minimal: false,
        status_bar: true,
        clock: Place::Side,
        pomodoro: Place::Side,
        face,
        hour24: true,
        clock_chip: true,
        pomodoro_chip: None,
        anchor: Anchor::Center,
        cell_aspect: 2.0,
    }
}

impl Case {
    fn items(&self) -> Vec<DockItem> {
        let forms = |place: Place, f: &dyn Fn(Place) -> Vec<_>| match place {
            Place::Off => Vec::new(),
            p => f(p),
        };
        vec![
            DockItem {
                place: self.clock,
                forms: forms(self.clock, &|p| clock_forms(self.face, self.hour24, p)),
                chip: self.clock_chip.then_some((5, 1)),
            },
            DockItem {
                place: self.pomodoro,
                forms: forms(self.pomodoro, &|p| pomodoro_forms(p, 5)),
                chip: self.pomodoro_chip.map(|w| (w, 2)),
            },
        ]
    }

    fn input<'a>(&self, items: &'a [DockItem]) -> LayoutInput<'a> {
        LayoutInput {
            minimal: self.minimal,
            status_bar: self.status_bar,
            dock: items,
            anchor: self.anchor,
            cell_aspect: self.cell_aspect,
        }
    }

    fn place(&self, widget: usize) -> Place {
        [self.clock, self.pomodoro][widget]
    }
}

fn at(cols: u16, rows: u16, case: &Case) -> Layout {
    let items = case.items();
    layout(Rect::new(0, 0, cols, rows), &case.input(&items))
}

fn blocks() -> &'static dyn Face {
    clock::face_by_name("blocks").unwrap()
}

/// The clock face's form and rects in a placed clock.
fn clock_of(l: &Layout, case: &Case) -> Option<(clock::Form, Rect, Option<Rect>)> {
    let p = l.placed(CLOCK)?;
    let align = if case.clock == Place::Side {
        ratatui::layout::Alignment::Left
    } else {
        case.anchor.align()
    };
    clock_parts(case.face, case.hour24, case.clock, p.form, p.rect, align)
}

/// Every invariant §1 promises, for one layout.
fn check(l: &Layout, case: &Case) {
    let area = l.area;
    let (cols, rows) = (area.width, area.height);
    let ctx = format!(
        "{cols}x{rows} minimal={} clock={:?} pomodoro={:?} {:?}",
        case.minimal, case.clock, case.pomodoro, case.anchor
    );
    let inside = |r: Rect, what: &str| {
        assert!(!r.is_empty(), "{ctx}: empty {what}");
        assert_eq!(r.intersection(area), r, "{ctx}: {what} {r:?} out of bounds");
    };

    let Some(lamp) = l.lamp else {
        assert!(cols < 4 || rows < 2, "{ctx}: no lamp");
        assert!(l.status.is_none() && l.panel.is_none() && l.chip.is_none());
        assert!(l.on_lava.is_none());
        return;
    };
    inside(lamp, "lamp");
    // No frame: the lamp starts in the top-left corner and spans the
    // screen's width unless the panel sits beside it.
    assert_eq!((lamp.x, lamp.y), (area.x, area.y), "{ctx}: lamp inset");
    if l.panel.as_ref().is_none_or(|p| p.rect.x < lamp.right()) {
        assert_eq!(lamp.width, cols, "{ctx}: lamp narrower than the screen");
    }

    let micro = !reaches(cols, rows, TINY);
    if let Some(s) = l.status {
        inside(s, "status");
        assert!(
            !case.minimal && rows >= 14 && cols >= 30,
            "{ctx}: status shown"
        );
        assert_eq!(s.height, 1);
        assert_eq!(s.y, rows - 1, "{ctx}: status not on the last row");
        assert!(!s.intersects(lamp), "{ctx}: status over lamp");
    } else {
        assert!(
            case.minimal || !case.status_bar || rows < 14 || cols < 30,
            "{ctx}: status missing"
        );
    }

    for (stack, place) in [(&l.panel, Place::Side), (&l.on_lava, Place::Overlay)] {
        let Some(stack) = stack else { continue };
        // A stack holds exactly the widgets put there, in order.
        let want: Vec<usize> = (0..2).filter(|&w| case.place(w) == place).collect();
        let got: Vec<usize> = stack.items.iter().map(|p| p.widget).collect();
        assert_eq!(got, want, "{ctx}: {place:?} stack");
        for (i, a) in stack.items.iter().enumerate() {
            inside(a.rect, "widget");
            assert_eq!(a.rect.height, a.form.size.height, "{ctx}");
            assert!(a.rect.width >= a.form.size.width, "{ctx}");
            if !a.form.fill {
                assert_eq!(a.rect.width, a.form.size.width, "{ctx}");
            }
            assert_eq!(
                a.rect.intersection(stack.rect),
                a.rect,
                "{ctx}: widget {a:?} outside {:?}",
                stack.rect
            );
            if a.form.needs.tall {
                assert!(rows >= 36, "{ctx}: date line too early");
            }
            if a.form.needs.huge {
                assert!(reaches(cols, rows, HUGE), "{ctx}: XL too early");
            }
            for b in &stack.items[i + 1..] {
                assert!(!a.rect.intersects(b.rect), "{ctx}: widgets overlap");
            }
        }
    }
    if let Some((form, face, date)) = clock_of(l, case) {
        assert_eq!(face.as_size(), form.size, "{ctx}");
        let seconds_ok = case.clock == Place::Side && form.tier >= Tier::L;
        assert!(!form.seconds || seconds_ok, "{ctx}: seconds {form:?}");
        if let Some(d) = date {
            assert!(rows >= 36, "{ctx}: date line too early");
            assert!(!d.intersects(face), "{ctx}: date over face");
        }
    }

    if let Some(p) = &l.panel {
        assert!(!case.minimal && !micro, "{ctx}: panel in minimal/micro");
        inside(p.rect, "panel");
        assert!(!p.rect.intersects(lamp), "{ctx}: panel over lamp");
        if let Some(s) = l.status {
            assert!(!p.rect.intersects(s), "{ctx}: panel over status");
        }
        for a in &p.items {
            assert!(
                a.rect.x > p.rect.x && a.rect.right() < p.rect.right(),
                "{ctx}: no side padding"
            );
            if a.widget == POMODORO {
                assert_eq!(a.rect.height, 3);
                assert!(a.rect.width >= 20, "{ctx}: pomodoro too narrow");
            }
        }
    }

    if let Some(s) = &l.on_lava {
        assert!(!micro, "{ctx}: widgets on a micro lamp");
        assert!(
            reaches(lamp.width, lamp.height, (28, 10)),
            "{ctx}: lamp too small"
        );
        // Never more than 60 % of the lamp's width or half its height,
        // and the backing stays inside the lamp, off its top row (toasts)
        // and its bottom row (the chip).
        assert!(
            u32::from(s.rect.width) * 5 <= u32::from(lamp.width) * 3,
            "{ctx}: too wide on the lava"
        );
        assert!(s.rect.height <= lamp.height / 2, "{ctx}: too tall");
        let back = halo(s.rect);
        let area = |r: Rect| u32::from(r.width) * u32::from(r.height);
        assert!(
            area(back) * 100 <= area(lamp) * 35,
            "{ctx}: covers the lamp"
        );
        assert_eq!(back.intersection(lamp), back, "{ctx}: backing off the lamp");
        assert!(back.y > lamp.y, "{ctx}: backing on the toast row");
        assert!(
            back.bottom() < lamp.bottom(),
            "{ctx}: backing on the chip row"
        );
        for other in [l.status, l.panel.as_ref().map(|p| p.rect), l.toast]
            .into_iter()
            .flatten()
        {
            assert!(!back.intersects(other), "{ctx}: lava stack over {other:?}");
        }
        if let Some(c) = l.chip {
            assert!(!back.intersects(c.rect), "{ctx}: lava stack over the chip");
        }
    }

    // The chip: the highest-ranked widget with no room where it was put.
    let homeless = |w: usize| match case.place(w) {
        Place::Side => l.panel.is_none(),
        Place::Overlay => l.on_lava.is_none(),
        Place::Off => false,
    };
    let chips = [
        case.clock_chip.then_some(1u8),
        case.pomodoro_chip.map(|_| 2u8),
    ];
    let want = (0..2)
        .filter(|&w| homeless(w))
        .filter_map(|w| chips[w].map(|rank| (rank, w)))
        .max()
        .map(|(_, w)| w);
    if let Some(c) = l.chip {
        inside(c.rect, "chip");
        assert!(!micro, "{ctx}: chip in micro");
        assert_eq!(c.rect.height, 1);
        assert_eq!(Some(c.widget), want, "{ctx}: wrong chip");
        if let Some(s) = l.status {
            assert!(!c.rect.intersects(s), "{ctx}: chip over status");
        }
        if let Some(p) = &l.panel {
            assert!(!c.rect.intersects(p.rect), "{ctx}: chip over panel");
        }
    } else {
        assert!(micro || want.is_none(), "{ctx}: chip missing");
    }

    if let Some(t) = l.toast {
        inside(t, "toast");
        assert!(cols >= 16 && rows >= 4);
        for other in [l.status, l.panel.as_ref().map(|p| p.rect)]
            .into_iter()
            .flatten()
        {
            assert!(!t.intersects(other), "{ctx}: toast row over {other:?}");
        }
    } else {
        assert!(cols < 16 || rows < 4, "{ctx}: toast row missing");
    }
}

/// Check every size in `cols × rows` for each case (items built once).
fn sweep(cols: std::ops::RangeInclusive<u16>, rows: std::ops::RangeInclusive<u16>, cases: &[Case]) {
    let built: Vec<(Case, Vec<DockItem>)> = cases.iter().map(|c| (*c, c.items())).collect();
    for c in cols {
        for r in rows.clone() {
            for (case, items) in &built {
                let l = layout(Rect::new(0, 0, c, r), &case.input(items));
                check(&l, case);
            }
        }
    }
}

#[test]
fn every_size_is_clean() {
    let faces = ["blocks", "analog", "words"].map(|n| clock::face_by_name(n).unwrap());
    let base = case(faces[0]);
    let variants = [
        base,
        Case {
            face: faces[1],
            hour24: false,
            ..base
        },
        Case {
            face: faces[2],
            ..base
        },
        Case {
            pomodoro_chip: Some(7),
            ..base
        },
        Case {
            clock: Place::Off,
            status_bar: false,
            clock_chip: false,
            ..base
        },
        Case {
            minimal: true,
            ..base
        },
        Case {
            minimal: true,
            pomodoro_chip: Some(9),
            ..base
        },
        Case {
            minimal: true,
            clock_chip: false,
            cell_aspect: 2.4,
            ..base
        },
    ];
    sweep(1..=300, 1..=100, &variants);
}

/// The dock's places mixed: widgets on the lava at every anchor, beside
/// side ones, in minimal mode, with chips, across 12×5 … 300×90.
#[test]
fn every_size_is_clean_with_widgets_on_the_lava() {
    let faces = ["blocks", "analog", "words", "segment", "binary", "text"]
        .map(|n| clock::face_by_name(n).unwrap());
    let base = Case {
        clock: Place::Overlay,
        ..case(faces[0])
    };
    let mut cases = Vec::new();
    for (i, anchor) in [
        Anchor::Center,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::Bottom,
        Anchor::BottomLeft,
        Anchor::TopLeft,
    ]
    .into_iter()
    .enumerate()
    {
        let face = faces[i % faces.len()];
        cases.extend([
            Case {
                anchor,
                face,
                ..base
            },
            Case {
                anchor,
                face,
                pomodoro: Place::Overlay,
                pomodoro_chip: Some(7),
                ..base
            },
            Case {
                anchor,
                face,
                clock: Place::Side,
                pomodoro: Place::Overlay,
                hour24: false,
                ..base
            },
            Case {
                anchor,
                face,
                pomodoro: Place::Overlay,
                minimal: true,
                ..base
            },
            Case {
                anchor,
                face,
                clock: Place::Off,
                clock_chip: false,
                pomodoro: Place::Overlay,
                pomodoro_chip: Some(12),
                cell_aspect: 2.4,
                ..base
            },
        ]);
    }
    sweep(12..=300, 5..=90, &cases);
}

#[test]
fn huge_sizes_are_clean() {
    let base = case(blocks());
    for (cols, rows) in [(400, 120), (500, 40), (60, 300), (1000, 1000)] {
        for minimal in [false, true] {
            for clock in [Place::Side, Place::Overlay] {
                let v = Case {
                    minimal,
                    clock,
                    pomodoro: clock,
                    ..base
                };
                check(&at(cols, rows, &v), &v);
            }
        }
    }
}

#[test]
fn layout_is_deterministic_and_stateless() {
    let v = case(blocks());
    assert_eq!(at(80, 24, &v), at(80, 24, &v));
    let _ = at(300, 100, &v);
    let items = v.items();
    assert_eq!(
        at(80, 24, &v),
        layout(Rect::new(0, 0, 80, 24), &v.input(&items))
    );
}

// --- the mockup sizes (§1.5) -----------------------------------------------

fn face_tier(l: &Layout, case: &Case) -> Option<(Tier, u16, u16)> {
    clock_of(l, case).map(|(f, ..)| (f.tier, f.size.width, f.size.height))
}

fn has_date(l: &Layout, case: &Case) -> bool {
    clock_of(l, case).is_some_and(|(.., date)| date.is_some())
}

#[test]
fn micro_is_lamp_only() {
    let l = at(16, 6, &case(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 16, 6)));
    assert!(l.status.is_none() && l.panel.is_none() && l.chip.is_none());
}

#[test]
fn tiny_is_the_lamp_with_a_corner_chip() {
    let l = at(20, 8, &case(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 20, 8)));
    assert!(l.status.is_none() && l.panel.is_none());
    let chip = l.chip.unwrap();
    assert_eq!(chip.rect, Rect::new(13, 7, 7, 1));
}

#[test]
fn small_50x16_has_status_and_chip_but_no_panel() {
    let l = at(50, 16, &case(blocks()));
    assert!(l.status.is_some());
    assert!(l.panel.is_none(), "lamp would keep only 56 % of the width");
    assert!(l.chip.is_some());
}

#[test]
fn small_wide_72x18_gets_a_right_panel() {
    let c = case(blocks());
    let l = at(72, 18, &c);
    assert_eq!(l.lamp.unwrap().width, 50);
    assert_eq!(face_tier(&l, &c), Some((Tier::M, 17, 3)));
    assert!(!has_date(&l, &c));
}

#[test]
fn medium_80x24_gets_a_right_panel() {
    let c = case(blocks());
    let l = at(80, 24, &c);
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 56, 23)));
    assert_eq!(l.panel.as_ref().unwrap().rect.x, 56);
    assert_eq!(face_tier(&l, &c), Some((Tier::M, 17, 3)));
    assert_eq!(l.status.unwrap(), Rect::new(2, 23, 76, 1));
}

#[test]
fn large_120x36_gets_l_face_and_date() {
    let c = case(blocks());
    let l = at(120, 36, &c);
    assert_eq!(face_tier(&l, &c), Some((Tier::L, 34, 5)));
    assert!(has_date(&l, &c));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 84, 35)));
}

#[test]
fn wide_160x22_has_the_full_panel_and_no_date() {
    let c = case(blocks());
    let l = at(160, 22, &c);
    assert_eq!(l.panel.as_ref().unwrap().rect.width, 36);
    assert!(!has_date(&l, &c));
}

#[test]
fn ultra_tall_34x56_puts_the_panel_below() {
    let l = at(34, 56, &case(blocks()));
    let lamp = l.lamp.unwrap();
    let p = l.panel.unwrap();
    assert_eq!(p.rect.y, lamp.bottom() + 1, "a blank row above the panel");
    assert_eq!(lamp.width, 34);
}

#[test]
fn huge_250x70_has_the_panel_and_date() {
    let c = case(blocks());
    let l = at(250, 70, &c);
    assert!(has_date(&l, &c));
    // The panel grows past 36 for the blocks XL face (51×8), no further.
    assert_eq!(face_tier(&l, &c), Some((Tier::XL, 51, 8)));
    assert_eq!(l.panel.unwrap().rect.width, 53);
}

#[test]
fn wide_but_short_gets_blocks_l_with_seconds() {
    // 200 cols lets the panel grow; 50 rows isn't Huge, so no XL.
    let c = case(blocks());
    let l = at(220, 50, &c);
    let (form, ..) = clock_of(&l, &c).unwrap();
    assert_eq!(
        (form.tier, form.seconds, form.size.width),
        (Tier::L, true, 54)
    );
    assert_eq!(l.panel.unwrap().rect.width, 56);
    // Below 200 cols the panel stays at 36 and seconds don't fit.
    let l = at(199, 50, &c);
    assert_eq!(l.panel.as_ref().unwrap().rect.width, 36);
    assert_eq!(face_tier(&l, &c), Some((Tier::L, 34, 5)));
}

#[test]
fn narrow_faces_keep_the_36_col_panel_when_huge() {
    let c = case(clock::face_by_name("analog").unwrap());
    let l = at(250, 70, &c);
    assert_eq!(face_tier(&l, &c), Some((Tier::XL, 31, 16)));
    assert_eq!(l.panel.unwrap().rect.width, 36);
}

#[test]
fn minimal_is_the_lamp_with_a_corner_chip() {
    let v = Case {
        minimal: true,
        ..case(blocks())
    };
    let l = at(80, 24, &v);
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 80, 24)));
    assert!(l.status.is_none() && l.panel.is_none());
    assert_eq!(l.chip.unwrap().rect, Rect::new(73, 23, 7, 1));
}

#[test]
fn minimal_off_hides_the_clock_but_not_the_pomodoro() {
    let off = Case {
        minimal: true,
        clock_chip: false,
        ..case(blocks())
    };
    assert!(at(60, 12, &off).chip.is_none());
    let pomo = Case {
        pomodoro_chip: Some(7),
        ..off
    };
    let chip = at(60, 12, &pomo)
        .chip
        .expect("a running pomodoro still shows");
    assert_eq!(chip.widget, POMODORO);
}

#[test]
fn hidden_clock_keeps_the_pomodoro_panel() {
    let v = Case {
        clock: Place::Off,
        ..case(blocks())
    };
    let p = at(80, 24, &v).panel.unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].widget, POMODORO);
    assert_eq!(p.rect.height, 3);
}

// --- widgets on the lava --------------------------------------------------

#[test]
fn both_on_the_lava_give_the_lamp_the_whole_width() {
    let c = Case {
        clock: Place::Overlay,
        pomodoro: Place::Overlay,
        ..case(blocks())
    };
    let l = at(80, 24, &c);
    assert!(l.panel.is_none() && l.chip.is_none());
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 80, 23)));
    let s = l.on_lava.as_ref().unwrap();
    // Blocks L over the full pomodoro, centred, a row apart.
    assert_eq!(face_tier(&l, &c), Some((Tier::L, 34, 5)));
    assert_eq!(s.rect, Rect::new(23, 7, 34, 9));
    assert_eq!(s.items[1].rect, Rect::new(23, 13, 34, 3));
}

#[test]
fn the_clock_on_the_lava_leaves_the_pomodoro_in_the_panel() {
    let c = Case {
        clock: Place::Overlay,
        ..case(blocks())
    };
    let l = at(80, 24, &c);
    let p = l.panel.as_ref().unwrap();
    assert_eq!((p.items.len(), p.items[0].widget), (1, POMODORO));
    assert_eq!(l.on_lava.as_ref().unwrap().items[0].widget, CLOCK);
    assert!(l.lamp.unwrap().width < 80);
}

#[test]
fn the_lava_stack_shrinks_then_falls_back_to_the_chip() {
    let c = Case {
        clock: Place::Overlay,
        ..case(blocks())
    };
    // The face shrinks with the lamp (at 80×24 the pomodoro's panel
    // leaves it 56 cols, too few for L's 34)…
    let tiers: Vec<_> = [(120, 36), (80, 24), (40, 14), (28, 10)]
        .map(|(w, h)| face_tier(&at(w, h, &c), &c).map(|t| t.0))
        .into();
    assert_eq!(
        tiers,
        [
            Some(Tier::L),
            Some(Tier::M),
            Some(Tier::M),
            Some(Tier::Text)
        ]
    );
    // …and on a lamp under 28×10, it's the chip.
    let l = at(27, 10, &c);
    assert!(l.on_lava.is_none());
    assert_eq!(l.chip.unwrap().widget, CLOCK);
}

#[test]
fn anchors_put_the_stack_where_they_say() {
    let base = Case {
        clock: Place::Overlay,
        ..case(blocks())
    };
    let lamp = |a| {
        let l = at(120, 36, &Case { anchor: a, ..base });
        (l.lamp.unwrap(), l.on_lava.unwrap().rect)
    };
    let (lamp_r, centre) = lamp(Anchor::Center);
    let mid = |r: Rect| (2 * r.x + r.width, 2 * r.y + r.height);
    let (lx, ly) = mid(lamp_r);
    let (cx, cy) = mid(centre);
    assert!(lx.abs_diff(cx) <= 1 && ly.abs_diff(cy) <= 1, "centred");
    let (_, tl) = lamp(Anchor::TopLeft);
    let (_, br) = lamp(Anchor::BottomRight);
    assert!(tl.x < centre.x && tl.y < centre.y);
    assert!(br.right() > centre.right() && br.bottom() > centre.bottom());
    let (_, top) = lamp(Anchor::Top);
    assert_eq!((top.x, top.y), (centre.x, tl.y));
}

#[test]
fn no_seconds_on_the_lava() {
    let c = Case {
        clock: Place::Overlay,
        ..case(blocks())
    };
    for (w, h) in [(220, 50), (300, 90)] {
        let (form, ..) = clock_of(&at(w, h, &c), &c).unwrap();
        assert!(!form.seconds, "{w}x{h}");
    }
}

// --- status bar & toasts ----------------------------------------------------

#[test]
fn hints_drop_in_spec_order() {
    let keys = |avail| -> String {
        fit_hints(HINTS, avail)
            .iter()
            .map(|h| h.0)
            .collect::<Vec<_>>()
            .join("")
    };
    assert_eq!(keys(200), "scpm␣?");
    // m, ␣, p, c, s go first; ? help last.
    let mut seen = Vec::new();
    for avail in (0..=200).rev() {
        let k = keys(avail);
        if seen.last() != Some(&k) {
            seen.push(k);
        }
    }
    assert_eq!(seen, ["scpm␣?", "scp␣?", "scp?", "sc?", "s?", "?", ""]);
}

#[test]
fn toasts_drop_whole_words() {
    assert_eq!(
        fit_words("braille  6/12", 20).as_deref(),
        Some("braille  6/12")
    );
    assert_eq!(fit_words("braille  6/12", 10).as_deref(), Some("braille"));
    assert_eq!(fit_words("break · 5:00", 8).as_deref(), Some("break"));
    assert_eq!(fit_words("braille", 5), None);
}

// --- snapshots ----------------------------------------------------------------

/// A layout as a character map: `L` lamp, `P` panel padding, `f` face,
/// `d` date, `o` pomodoro, `~` the backing of the widgets on the lava
/// (their own cells as in the panel), `S` status, `c` chip, `t` toast
/// row, `.` background.
fn picture(l: &Layout, case: &Case) -> String {
    let (w, h) = (usize::from(l.area.width), usize::from(l.area.height));
    let mut grid = vec![vec!['.'; w]; h];
    let mut fill = |r: Rect, ch: char| {
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                grid[usize::from(y)][usize::from(x)] = ch;
            }
        }
    };
    if let Some(t) = l.toast {
        fill(t, 't');
    }
    if let Some(lamp) = l.lamp {
        fill(lamp, 'L');
    }
    if let Some(p) = &l.panel {
        fill(p.rect, 'P');
    }
    if let Some(s) = &l.on_lava {
        fill(halo(s.rect), '~');
    }
    for p in [&l.panel, &l.on_lava].into_iter().flatten() {
        for item in &p.items {
            if item.widget == POMODORO {
                fill(item.rect, 'o');
            }
        }
    }
    if let Some((_, face, date)) = clock_of(l, case) {
        fill(face, 'f');
        if let Some(d) = date {
            fill(d, 'd');
        }
    }
    if let Some(s) = l.status {
        fill(s, 'S');
    }
    if let Some(c) = l.chip {
        fill(c.rect, 'c');
    }
    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>() + "\n")
        .collect()
}

#[test]
fn snapshots_at_mockup_sizes() {
    let full = case(blocks());
    let minimal = Case {
        minimal: true,
        ..full
    };
    let lava = Case {
        clock: Place::Overlay,
        pomodoro: Place::Overlay,
        ..full
    };
    let cases = [
        ("16x6", 16, 6, full),
        ("20x8", 20, 8, full),
        ("50x16", 50, 16, full),
        ("72x18", 72, 18, full),
        ("80x24", 80, 24, full),
        ("120x36", 120, 36, full),
        ("160x22", 160, 22, full),
        ("250x70", 250, 70, full),
        ("34x56", 34, 56, full),
        ("minimal_80x24", 80, 24, minimal),
        ("lava_30x10", 30, 10, lava),
        ("lava_80x24", 80, 24, lava),
        (
            "lava_clock_120x36",
            120,
            36,
            Case {
                pomodoro: Place::Side,
                ..lava
            },
        ),
        (
            "lava_top_right_160x40",
            160,
            40,
            Case {
                anchor: Anchor::TopRight,
                ..lava
            },
        ),
        (
            "lava_minimal_80x24",
            80,
            24,
            Case {
                minimal: true,
                anchor: Anchor::BottomLeft,
                ..lava
            },
        ),
    ];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/snapshots");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    for (name, cols, rows, v) in cases {
        let got = picture(&at(cols, rows, &v), &v);
        let path = dir.join(format!("layout_{name}.txt"));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &got).unwrap();
        } else {
            let want = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("missing {path:?}: run UPDATE_SNAPSHOTS=1 cargo test"));
            assert_eq!(
                got, want,
                "layout {name} changed (UPDATE_SNAPSHOTS=1 to accept)"
            );
        }
    }
}
