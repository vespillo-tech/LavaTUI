//! Layout tests (docs/design.md §1.1): an exhaustive size sweep checking
//! the invariants, the mockup sizes checked element by element, and text
//! snapshots of the mockup layouts (`UPDATE_SNAPSHOTS=1 cargo test` to
//! rewrite `ui/snapshots/`, then review the diff).

use ratatui::layout::Rect;

use super::chrome::{HINTS, fit_hints, fit_words};
use super::layout::*;
use crate::clock::{self, Face, Tier};
use crate::config::MinimalClock;

fn input(face: &dyn Face) -> LayoutInput<'_> {
    LayoutInput {
        minimal: false,
        status_bar: true,
        show_clock: true,
        face,
        hour24: true,
        chip: Some((ChipKind::Clock, 5)),
        minimal_clock: MinimalClock::Corner,
        cell_aspect: 2.0,
    }
}

fn at(cols: u16, rows: u16, input: &LayoutInput) -> Layout {
    layout(Rect::new(0, 0, cols, rows), input)
}

fn blocks() -> &'static dyn Face {
    clock::face_by_name("blocks").unwrap()
}

/// Every invariant §1 promises, for one layout.
fn check(l: &Layout, input: &LayoutInput) {
    let area = l.area;
    let (cols, rows) = (area.width, area.height);
    let ctx = format!("{cols}x{rows} minimal={}", input.minimal);
    let inside = |r: Rect, what: &str| {
        assert!(!r.is_empty(), "{ctx}: empty {what}");
        assert_eq!(r.intersection(area), r, "{ctx}: {what} {r:?} out of bounds");
    };

    let Some(lamp) = l.lamp else {
        assert!(cols < 4 || rows < 2, "{ctx}: no lamp");
        assert!(l.status.is_none() && l.panel.is_none() && l.chip.is_none());
        return;
    };
    inside(lamp, "lamp");
    // No frame: the lamp starts in the top-left corner and spans the
    // screen's width unless the panel sits beside it.
    assert_eq!((lamp.x, lamp.y), (area.x, area.y), "{ctx}: lamp inset");
    if l.panel.is_none_or(|p| p.rect.x < lamp.right()) {
        assert_eq!(lamp.width, cols, "{ctx}: lamp narrower than the screen");
    }

    let micro = !reaches(cols, rows, TINY);
    if let Some(s) = l.status {
        inside(s, "status");
        assert!(
            !input.minimal && rows >= 14 && cols >= 30,
            "{ctx}: status shown"
        );
        assert_eq!(s.height, 1);
        assert_eq!(s.y, rows - 1, "{ctx}: status not on the last row");
        assert!(!s.intersects(lamp), "{ctx}: status over lamp");
    } else {
        assert!(
            input.minimal || !input.status_bar || rows < 14 || cols < 30,
            "{ctx}: status missing"
        );
    }

    if let Some(p) = l.panel {
        assert!(!input.minimal && !micro, "{ctx}: panel in minimal/micro");
        inside(p.rect, "panel");
        assert!(!p.rect.intersects(lamp), "{ctx}: panel over lamp");
        if let Some(s) = l.status {
            assert!(!p.rect.intersects(s), "{ctx}: panel over status");
        }
        let mut parts = vec![p.pomodoro];
        assert_eq!(p.pomodoro.height, 3);
        assert!(p.pomodoro.width >= 20, "{ctx}: pomodoro too narrow");
        if let Some((r, form)) = p.face {
            assert_eq!(r.as_size(), form.size, "{ctx}");
            assert!(input.show_clock);
            assert!(
                !form.seconds || form.tier >= Tier::L,
                "{ctx}: seconds below L"
            );
            parts.push(r);
        } else {
            assert!(!input.show_clock, "{ctx}: face missing");
        }
        if let Some(d) = p.date {
            assert!(rows >= 36, "{ctx}: date line too early");
            parts.push(d);
        }
        for (i, a) in parts.iter().enumerate() {
            assert_eq!(
                a.intersection(p.rect),
                *a,
                "{ctx}: panel part {a:?} outside {:?}",
                p.rect
            );
            assert!(
                a.x > p.rect.x && a.right() < p.rect.right(),
                "{ctx}: no side padding"
            );
            for b in &parts[i + 1..] {
                assert!(!a.intersects(*b), "{ctx}: panel parts overlap");
            }
        }
        assert!(l.chip.is_none(), "{ctx}: chip and panel");
    }

    if let Some(c) = l.chip {
        inside(c.rect, "chip");
        assert!(!micro, "{ctx}: chip in micro");
        assert_eq!(c.rect.height, 1);
        if let Some(s) = l.status {
            assert!(!c.rect.intersects(s), "{ctx}: chip over status");
        }
    }

    if let Some(t) = l.toast {
        inside(t, "toast");
        assert!(cols >= 16 && rows >= 4);
        for other in [l.status, l.panel.map(|p| p.rect)].into_iter().flatten() {
            assert!(!t.intersects(other), "{ctx}: toast row over {other:?}");
        }
    } else {
        assert!(cols < 16 || rows < 4, "{ctx}: toast row missing");
    }
}

#[test]
fn every_size_is_clean() {
    let faces = ["blocks", "analog", "words"].map(|n| clock::face_by_name(n).unwrap());
    let base = input(faces[0]);
    let variants = [
        base,
        LayoutInput {
            face: faces[1],
            hour24: false,
            ..base
        },
        LayoutInput {
            face: faces[2],
            ..base
        },
        LayoutInput {
            chip: Some((ChipKind::Pomodoro, 7)),
            ..base
        },
        LayoutInput {
            show_clock: false,
            status_bar: false,
            chip: None,
            ..base
        },
        LayoutInput {
            minimal: true,
            ..base
        },
        LayoutInput {
            minimal: true,
            chip: Some((ChipKind::Pomodoro, 9)),
            ..base
        },
        LayoutInput {
            minimal: true,
            minimal_clock: MinimalClock::Off,
            cell_aspect: 2.4,
            ..base
        },
    ];
    for cols in 1..=300 {
        for rows in 1..=100 {
            for v in &variants {
                check(&at(cols, rows, v), v);
            }
        }
    }
}

#[test]
fn huge_sizes_are_clean() {
    let base = input(blocks());
    for (cols, rows) in [(400, 120), (500, 40), (60, 300), (1000, 1000)] {
        for minimal in [false, true] {
            let v = LayoutInput { minimal, ..base };
            check(&at(cols, rows, &v), &v);
        }
    }
}

#[test]
fn layout_is_deterministic_and_stateless() {
    let v = input(blocks());
    assert_eq!(at(80, 24, &v), at(80, 24, &v));
    let _ = at(300, 100, &v);
    assert_eq!(at(80, 24, &v), layout(Rect::new(0, 0, 80, 24), &v));
}

// --- the mockup sizes (§1.5) -----------------------------------------------

fn face_tier(l: &Layout) -> Option<(Tier, u16, u16)> {
    l.panel?
        .face
        .map(|(_, f)| (f.tier, f.size.width, f.size.height))
}

#[test]
fn micro_is_lamp_only() {
    let l = at(16, 6, &input(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 16, 6)));
    assert!(l.status.is_none() && l.panel.is_none() && l.chip.is_none());
}

#[test]
fn tiny_is_the_lamp_with_a_corner_chip() {
    let l = at(20, 8, &input(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 20, 8)));
    assert!(l.status.is_none() && l.panel.is_none());
    let chip = l.chip.unwrap();
    assert_eq!(chip.rect, Rect::new(13, 7, 7, 1));
}

#[test]
fn small_50x16_has_status_and_chip_but_no_panel() {
    let l = at(50, 16, &input(blocks()));
    assert!(l.status.is_some());
    assert!(l.panel.is_none(), "lamp would keep only 56 % of the width");
    assert!(l.chip.is_some());
}

#[test]
fn small_wide_72x18_gets_a_right_panel() {
    let l = at(72, 18, &input(blocks()));
    assert_eq!(l.lamp.unwrap().width, 50);
    assert_eq!(face_tier(&l), Some((Tier::M, 17, 3)));
    assert!(l.panel.unwrap().date.is_none());
}

#[test]
fn medium_80x24_gets_a_right_panel() {
    let l = at(80, 24, &input(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 56, 23)));
    assert_eq!(l.panel.unwrap().rect.x, 56);
    assert_eq!(face_tier(&l), Some((Tier::M, 17, 3)));
    assert_eq!(l.status.unwrap(), Rect::new(2, 23, 76, 1));
}

#[test]
fn large_120x36_gets_l_face_and_date() {
    let l = at(120, 36, &input(blocks()));
    assert_eq!(face_tier(&l), Some((Tier::L, 34, 5)));
    assert!(l.panel.unwrap().date.is_some());
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 84, 35)));
}

#[test]
fn wide_160x22_has_the_full_panel_and_no_date() {
    let l = at(160, 22, &input(blocks()));
    let p = l.panel.unwrap();
    assert_eq!(p.rect.width, 36);
    assert!(p.date.is_none());
}

#[test]
fn ultra_tall_34x56_puts_the_panel_below() {
    let l = at(34, 56, &input(blocks()));
    let lamp = l.lamp.unwrap();
    let p = l.panel.unwrap();
    assert_eq!(p.rect.y, lamp.bottom() + 1, "a blank row above the panel");
    assert_eq!(lamp.width, 34);
}

#[test]
fn huge_250x70_has_the_panel_and_date() {
    let l = at(250, 70, &input(blocks()));
    let p = l.panel.unwrap();
    assert!(p.date.is_some());
    // The panel grows past 36 for the blocks XL face (51×8), no further.
    assert_eq!(face_tier(&l), Some((Tier::XL, 51, 8)));
    assert_eq!(p.rect.width, 53);
}

#[test]
fn wide_but_short_gets_blocks_l_with_seconds() {
    // 200 cols lets the panel grow; 50 rows isn't Huge, so no XL.
    let l = at(220, 50, &input(blocks()));
    let p = l.panel.unwrap();
    let (_, form) = p.face.unwrap();
    assert_eq!(
        (form.tier, form.seconds, form.size.width),
        (Tier::L, true, 54)
    );
    assert_eq!(p.rect.width, 56);
    // Below 200 cols the panel stays at 36 and seconds don't fit.
    let l = at(199, 50, &input(blocks()));
    assert_eq!(l.panel.unwrap().rect.width, 36);
    assert_eq!(face_tier(&l), Some((Tier::L, 34, 5)));
}

#[test]
fn narrow_faces_keep_the_36_col_panel_when_huge() {
    let l = at(250, 70, &input(clock::face_by_name("analog").unwrap()));
    assert_eq!(face_tier(&l), Some((Tier::XL, 31, 16)));
    assert_eq!(l.panel.unwrap().rect.width, 36);
}

#[test]
fn minimal_is_the_lamp_with_a_corner_chip() {
    let v = LayoutInput {
        minimal: true,
        ..input(blocks())
    };
    let l = at(80, 24, &v);
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 80, 24)));
    assert!(l.status.is_none() && l.panel.is_none());
    assert_eq!(l.chip.unwrap().rect, Rect::new(73, 23, 7, 1));
}

#[test]
fn minimal_off_hides_the_clock_but_not_the_pomodoro() {
    let v = LayoutInput {
        minimal: true,
        ..input(blocks())
    };
    let off = LayoutInput {
        minimal_clock: MinimalClock::Off,
        ..v
    };
    assert!(at(60, 12, &off).chip.is_none());
    let pomo = LayoutInput {
        chip: Some((ChipKind::Pomodoro, 7)),
        ..off
    };
    assert!(
        at(60, 12, &pomo).chip.is_some(),
        "a running pomodoro still shows"
    );
}

#[test]
fn hidden_clock_keeps_the_pomodoro_panel() {
    let v = LayoutInput {
        show_clock: false,
        ..input(blocks())
    };
    let p = at(80, 24, &v).panel.unwrap();
    assert!(p.face.is_none() && p.date.is_none());
    assert_eq!(p.rect.height, 3);
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

/// A layout as a character map: `L` lamp, `P`
/// panel padding, `f` face, `d` date, `o` pomodoro, `S` status, `c` chip,
/// `t` toast row, `.` background.
fn picture(l: &Layout) -> String {
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
    if let Some(p) = l.panel {
        fill(p.rect, 'P');
        if let Some((r, _)) = p.face {
            fill(r, 'f');
        }
        if let Some(d) = p.date {
            fill(d, 'd');
        }
        fill(p.pomodoro, 'o');
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
    let full = input(blocks());
    let minimal = LayoutInput {
        minimal: true,
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
    ];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/snapshots");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    for (name, cols, rows, v) in cases {
        let got = picture(&at(cols, rows, &v));
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
