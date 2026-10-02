//! Layout tests (docs/design.md §1.1, §4.6): exhaustive size sweeps
//! checking the invariants with up to four widgets in every place and many
//! anchor mixes, the mockup sizes checked element by element, and text
//! snapshots of the mockup layouts (`UPDATE_SNAPSHOTS=1 cargo test` to
//! rewrite `ui/snapshots/`, then review the diff).

use ratatui::layout::{Alignment, Rect};

use super::chrome::{HINTS, fit_hints, fit_words};
use super::layout::*;
use crate::clock::{self, Face, Tier};
use crate::dock::cover::{CoverSize, Show as CoverShow, cover_forms};
use crate::dock::{
    Anchor, LyricsShow, Place, Show, WidgetForm, clock_forms, clock_parts, lyrics_forms,
    music_forms, pomodoro_forms,
};

/// A dock setup to lay out: where the clock (index 0), the pomodoro (1),
/// music (2), the lyrics widget (3, only when `lyrics`) and the cover (4,
/// only when placed) go and at which anchors, the face, and which chips
/// and ranks they have.
#[derive(Clone, Copy)]
struct Case {
    minimal: bool,
    status_bar: bool,
    places: [Place; 5],
    anchors: [Anchor; 5],
    lyrics: bool,
    cover_size: CoverSize,
    face: &'static dyn Face,
    hour24: bool,
    /// The clock's chip (none in minimal with `minimal.clock = "off"`).
    clock_chip: bool,
    /// A running pomodoro's chip width (rank 3); `None` while idle (0).
    pomodoro_chip: Option<u16>,
    /// Music has a track playing (chip, rank 2; lyrics too), else it
    /// shows its longest message (rank 0).
    music_track: bool,
    music_cover: bool,
    cell_aspect: f64,
}

const CLOCK: usize = 0;
const POMODORO: usize = 1;
const COVER: usize = 4;
/// The longest thing the music widget says instead of a track.
const PERMISSION: &str = "Allow control of Spotify: System Settings › Privacy & \
    Security › Automation › your terminal › Spotify";

fn case(face: &'static dyn Face) -> Case {
    Case {
        minimal: false,
        status_bar: true,
        places: [Place::Side, Place::Side, Place::Off, Place::Off, Place::Off],
        anchors: [
            Anchor::Center,
            Anchor::Center,
            Anchor::TopLeft,
            Anchor::Bottom,
            Anchor::TopRight,
        ],
        lyrics: false,
        cover_size: CoverSize::Medium,
        face,
        hour24: true,
        clock_chip: true,
        pomodoro_chip: None,
        music_track: true,
        music_cover: true,
        cell_aspect: 2.0,
    }
}

impl Case {
    fn widgets(&self) -> usize {
        if self.places[COVER] != Place::Off {
            5
        } else if self.lyrics {
            4
        } else {
            3
        }
    }

    fn items(&self) -> Vec<DockItem> {
        let forms = |place: Place, f: &dyn Fn(Place) -> Vec<WidgetForm>| match place {
            Place::Off => Vec::new(),
            p => f(p),
        };
        let p = self.places;
        let mut items = vec![
            DockItem {
                place: p[0],
                anchor: self.anchors[0],
                forms: forms(p[0], &|p| clock_forms(self.face, self.hour24, p)),
                chip: self.clock_chip.then_some(5),
                rank: 1,
            },
            DockItem {
                place: p[1],
                anchor: self.anchors[1],
                forms: forms(p[1], &|p| pomodoro_forms(p, 5)),
                chip: self.pomodoro_chip,
                rank: if self.pomodoro_chip.is_some() { 3 } else { 0 },
            },
            DockItem {
                place: p[2],
                anchor: self.anchors[2],
                forms: forms(p[2], &|p| music_forms(&self.show(), p)),
                chip: self.music_track.then_some(24),
                rank: if self.music_track { 2 } else { 0 },
            },
        ];
        if self.lyrics || p[COVER] != Place::Off {
            items.push(DockItem {
                place: if self.lyrics { p[3] } else { Place::Off },
                anchor: self.anchors[3],
                forms: forms(p[3], &|p| {
                    lyrics_forms(&LyricsShow::Lines { widest: 44 }, p)
                }),
                chip: self.music_track.then_some(20),
                rank: if self.music_track { 2 } else { 0 },
            });
        }
        if p[COVER] != Place::Off {
            let show = if self.music_track {
                CoverShow::Picture
            } else {
                CoverShow::Message("nothing playing".into())
            };
            items.push(DockItem {
                place: p[COVER],
                anchor: self.anchors[COVER],
                forms: cover_forms(&show, p[COVER], self.cover_size, self.cell_aspect),
                chip: None,
                rank: if self.music_track { 2 } else { 0 },
            });
        }
        items
    }

    fn show(&self) -> Show {
        if self.music_track {
            Show::Track {
                cover: self.music_cover,
                playing: true,
                line_w: 30,
            }
        } else {
            Show::Message(format!("{}{PERMISSION}", crate::dock::music::RICH.note))
        }
    }

    fn input<'a>(&self, items: &'a [DockItem]) -> LayoutInput<'a> {
        LayoutInput {
            minimal: self.minimal,
            status_bar: self.status_bar,
            dock: items,
            cell_aspect: self.cell_aspect,
        }
    }

    fn place(&self, widget: usize) -> Place {
        if widget < self.widgets() {
            self.places[widget]
        } else {
            Place::Off
        }
    }

    /// Priority: rank, then registry order (higher is more important).
    fn priority(items: &[DockItem], w: usize) -> (u8, std::cmp::Reverse<usize>) {
        (items[w].rank, std::cmp::Reverse(w))
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
    let place = case.places[CLOCK];
    let align = if place == Place::Side {
        Alignment::Left
    } else {
        case.anchors[CLOCK].align()
    };
    clock_parts(case.face, case.hour24, place, p.form, p.rect, align)
}

/// Every invariant §1 and §4.6 promise, for one layout.
fn check(l: &Layout, case: &Case) {
    let items = case.items();
    check_items(l, case, &items);
}

fn check_items(l: &Layout, case: &Case, items: &[DockItem]) {
    let area = l.area;
    let (cols, rows) = (area.width, area.height);
    let ctx = format!(
        "{cols}x{rows} minimal={} places={:?} anchors={:?}",
        case.minimal,
        &case.places[..case.widgets()],
        &case.anchors[..case.widgets()]
    );
    let inside = |r: Rect, what: &str| {
        assert!(!r.is_empty(), "{ctx}: empty {what}");
        assert_eq!(r.intersection(area), r, "{ctx}: {what} {r:?} out of bounds");
    };

    let Some(lamp) = l.lamp else {
        assert!(cols < 4 || rows < 2, "{ctx}: no lamp");
        assert!(l.status.is_none() && l.panel.is_none() && l.chips.is_none());
        assert!(l.on_lava.is_empty());
        return;
    };
    inside(lamp, "lamp");
    // No frame: the lamp starts in the top-left corner and spans the
    // screen's width unless the panel sits beside it.
    assert_eq!((lamp.x, lamp.y), (area.x, area.y), "{ctx}: lamp inset");
    if l.panel.as_ref().is_none_or(|p| p.rect.x < lamp.right()) {
        assert_eq!(lamp.width, cols, "{ctx}: lamp narrower than the screen");
    }
    // The lamp stays the hero: ≥ 60 % of the side the panel takes.
    if let Some(p) = &l.panel {
        if p.rect.x >= lamp.right() {
            assert!(u32::from(lamp.width) * 10 >= u32::from(cols) * 6, "{ctx}");
        } else {
            let content = rows - u16::from(l.status.is_some());
            assert!(
                u32::from(lamp.height) * 10 >= u32::from(content) * 6,
                "{ctx}"
            );
        }
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

    // Every widget is in at most one place, the one it was put, in a form
    // it offers there; widgets never overlap.
    let mut seen: Vec<Rect> = Vec::new();
    let stacks = l
        .panel
        .iter()
        .map(|s| (s, Place::Side))
        .chain(l.on_lava.iter().map(|s| (s, Place::Overlay)));
    for (stack, place) in stacks {
        let got: Vec<usize> = stack.items.iter().map(|p| p.widget).collect();
        let mut sorted = got.clone();
        sorted.sort_unstable();
        assert_eq!(got, sorted, "{ctx}: {place:?} stack out of registry order");
        for a in &stack.items {
            assert_eq!(
                case.place(a.widget),
                place,
                "{ctx}: widget {} misplaced",
                a.widget
            );
            assert!(
                items[a.widget].forms.contains(&a.form),
                "{ctx}: foreign form"
            );
            if place == Place::Overlay {
                assert_eq!(
                    stack.align,
                    case.anchors[a.widget].align(),
                    "{ctx}: widget {} at another anchor's stack",
                    a.widget
                );
            }
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
            assert!(
                seen.iter().all(|r| !r.intersects(a.rect)),
                "{ctx}: widgets overlap at {a:?}"
            );
            seen.push(a.rect);
        }
    }
    for w in 0..case.widgets() {
        let n = l
            .panel
            .iter()
            .chain(&l.on_lava)
            .flat_map(|s| &s.items)
            .filter(|p| p.widget == w)
            .count();
        assert!(n <= 1, "{ctx}: widget {w} placed {n} times");
    }
    if let Some((form, face, date)) = clock_of(l, case) {
        assert_eq!(face.as_size(), form.size, "{ctx}");
        let seconds_ok = case.places[CLOCK] == Place::Side && form.tier >= Tier::L;
        assert!(!form.seconds || seconds_ok, "{ctx}: seconds {form:?}");
        if let Some(d) = date {
            assert!(rows >= 36, "{ctx}: date line too early");
            assert!(!d.intersects(face), "{ctx}: date over face");
        }
    }

    // Dropped by rank: a widget left out of its place means every less
    // important one there is left out too.
    let placed = |w: usize| l.placed(w).is_some();
    for place in [Place::Side, Place::Overlay] {
        let there = place == Place::Side && l.panel.is_some()
            || place == Place::Overlay && !l.on_lava.is_empty();
        if !there {
            continue;
        }
        for a in (0..case.widgets()).filter(|&w| case.place(w) == place) {
            for b in (0..case.widgets()).filter(|&w| case.place(w) == place) {
                let room = |w: usize| !items[w].forms.is_empty();
                if room(a)
                    && room(b)
                    && !placed(a)
                    && Case::priority(items, b) < Case::priority(items, a)
                {
                    assert!(!placed(b), "{ctx}: {b} placed while {a}, above it, isn't");
                }
            }
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

    if !l.on_lava.is_empty() {
        assert!(!micro, "{ctx}: widgets on a micro lamp");
        assert!(
            reaches(lamp.width, lamp.height, (28, 10)),
            "{ctx}: lamp too small"
        );
        let area_of = |r: Rect| u32::from(r.width) * u32::from(r.height);
        let mut covered = 0;
        for (i, s) in l.on_lava.iter().enumerate() {
            // Never more than 60 % of the lamp's width or half its
            // height, and the backing stays inside the lamp, off its top
            // row (toasts) and its bottom row (the chips).
            assert!(
                u32::from(s.rect.width) * 5 <= u32::from(lamp.width) * 3,
                "{ctx}: too wide on the lava"
            );
            assert!(s.rect.height <= lamp.height / 2, "{ctx}: too tall");
            let back = halo(s.rect);
            covered += area_of(back);
            assert_eq!(back.intersection(lamp), back, "{ctx}: backing off the lamp");
            assert!(back.y > lamp.y, "{ctx}: backing on the toast row");
            assert!(
                back.bottom() < lamp.bottom(),
                "{ctx}: backing on the chip row"
            );
            for other in &l.on_lava[i + 1..] {
                assert!(!back.intersects(halo(other.rect)), "{ctx}: backings touch");
            }
            for other in [l.status, l.panel.as_ref().map(|p| p.rect), l.toast]
                .into_iter()
                .flatten()
            {
                assert!(!back.intersects(other), "{ctx}: lava stack over {other:?}");
            }
            if let Some(c) = &l.chips {
                assert!(!back.intersects(c.rect), "{ctx}: lava stack over the chips");
            }
        }
        assert!(
            covered * 100 <= area_of(lamp) * 35,
            "{ctx}: covers the lamp"
        );
    }

    // The chip row: the homeless widgets' chips, in registry order, the
    // least important left out first, inside the lamp's bottom row.
    let homeless: Vec<usize> = (0..case.widgets())
        .filter(|&w| case.place(w) != Place::Off && !placed(w))
        .filter(|&w| items[w].chip.is_some())
        .collect();
    if let Some(c) = &l.chips {
        inside(c.rect, "chip row");
        assert!(!micro, "{ctx}: chips in micro");
        assert_eq!(c.rect.height, 1);
        assert_eq!(
            c.rect.bottom(),
            lamp.bottom(),
            "{ctx}: chips off the corner"
        );
        assert_eq!(c.rect.right(), lamp.right(), "{ctx}: chips off the corner");
        let shown: Vec<usize> = c.items.iter().map(|i| i.widget).collect();
        let mut sorted = shown.clone();
        sorted.sort_unstable();
        assert_eq!(shown, sorted, "{ctx}: chips out of order");
        assert!(!shown.is_empty());
        for (i, chip) in c.items.iter().enumerate() {
            assert!(
                homeless.contains(&chip.widget),
                "{ctx}: chip for a placed widget"
            );
            assert_eq!(chip.rect.intersection(c.rect), chip.rect, "{ctx}");
            assert_eq!(Some(chip.rect.width), items[chip.widget].chip, "{ctx}");
            if let Some(next) = c.items.get(i + 1) {
                assert_eq!(next.rect.x, chip.rect.right() + CHIP_SEP, "{ctx}");
            }
        }
        let alone = |w: usize| items[w].chip.unwrap() + 2 <= lamp.width;
        for &h in homeless
            .iter()
            .filter(|&&h| !shown.contains(&h) && alone(h))
        {
            for &s in &shown {
                assert!(
                    Case::priority(items, h) < Case::priority(items, s),
                    "{ctx}: chip {s} shown over {h}"
                );
            }
        }
        if let Some(s) = l.status {
            assert!(!c.rect.intersects(s), "{ctx}: chips over status");
        }
        if let Some(p) = &l.panel {
            assert!(!c.rect.intersects(p.rect), "{ctx}: chips over panel");
        }
    } else {
        let fits = homeless
            .iter()
            .any(|&w| items[w].chip.unwrap() + 2 <= lamp.width);
        assert!(micro || !fits, "{ctx}: chips missing");
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

/// `base` with the first four places and anchors given (lyrics on when
/// its place isn't off); the cover keeps `base`'s, unless a fifth anchor
/// is given.
fn with(base: Case, places: [Place; 4], anchors: impl Into<Vec<Anchor>>) -> Case {
    let mut out = Case {
        lyrics: places[3] != Place::Off,
        ..base
    };
    out.places[..4].copy_from_slice(&places);
    for (i, a) in anchors.into().into_iter().enumerate().take(5) {
        out.anchors[i] = a;
    }
    out
}

/// `base` with the cover at `place`, `anchor`.
fn cover(base: Case, place: Place, anchor: Anchor) -> Case {
    let mut out = base;
    out.places[COVER] = place;
    out.anchors[COVER] = anchor;
    out
}

const SIDE: Place = Place::Side;
const LAVA: Place = Place::Overlay;
const OFF: Place = Place::Off;

#[test]
fn every_size_is_clean() {
    let faces = ["blocks", "analog", "words"].map(|n| clock::face_by_name(n).unwrap());
    let base = case(faces[0]);
    let a = base.anchors;
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
            status_bar: false,
            clock_chip: false,
            ..with(base, [OFF, SIDE, OFF, OFF], a)
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
        with(base, [SIDE, SIDE, SIDE, OFF], a),
        Case {
            music_track: false,
            pomodoro_chip: Some(7),
            ..with(base, [SIDE, SIDE, SIDE, OFF], a)
        },
        Case {
            minimal: true,
            ..with(base, [SIDE, SIDE, SIDE, SIDE], a)
        },
        with(base, [SIDE, SIDE, SIDE, SIDE], a),
        cover(
            with(base, [SIDE, SIDE, SIDE, OFF], a),
            SIDE,
            Anchor::TopRight,
        ),
        Case {
            cover_size: CoverSize::Fill,
            ..cover(base, SIDE, Anchor::TopRight)
        },
        Case {
            minimal: true,
            cover_size: CoverSize::Large,
            ..cover(
                with(base, [SIDE, SIDE, SIDE, OFF], a),
                SIDE,
                Anchor::TopRight,
            )
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
    let mut cases = Vec::new();
    for (i, anchor) in Anchor::ALL.into_iter().enumerate() {
        let face = faces[i % faces.len()];
        let next = Anchor::ALL[(i + 3) % Anchor::ALL.len()];
        let one = [anchor; 4];
        let base = case(face);
        cases.extend([
            with(base, [LAVA, SIDE, OFF, OFF], one),
            Case {
                pomodoro_chip: Some(7),
                ..with(base, [LAVA, LAVA, OFF, OFF], one)
            },
            Case {
                hour24: false,
                ..with(base, [SIDE, LAVA, OFF, OFF], one)
            },
            Case {
                minimal: true,
                ..with(base, [LAVA, LAVA, OFF, OFF], one)
            },
            Case {
                clock_chip: false,
                pomodoro_chip: Some(12),
                cell_aspect: 2.4,
                ..with(base, [OFF, LAVA, OFF, OFF], one)
            },
            with(base, [SIDE, SIDE, LAVA, OFF], one),
            Case {
                music_cover: i % 2 == 0,
                music_track: i % 3 != 0,
                ..with(base, [LAVA, LAVA, LAVA, OFF], [anchor, next, anchor, next])
            },
            Case {
                pomodoro_chip: Some(7),
                ..with(base, [LAVA, LAVA, LAVA, LAVA], [anchor, next, next, anchor])
            },
            cover(with(base, [SIDE, SIDE, LAVA, OFF], one), LAVA, next),
            Case {
                cover_size: [CoverSize::Small, CoverSize::Large, CoverSize::Fill][i % 3],
                music_track: i % 4 != 3,
                ..cover(with(base, [LAVA, SIDE, OFF, LAVA], one), LAVA, anchor)
            },
        ]);
    }
    sweep(12..=300, 5..=90, &cases);
}

/// Four widgets in every combination of places, under three anchor
/// patterns (all together, the defaults spread out, crowded corners) and
/// alternating ranks, on a coarse grid of sizes.
#[test]
fn every_place_and_anchor_mix_is_clean() {
    use Anchor::*;
    let patterns = [
        [Center; 4],
        [Center, Center, TopLeft, Bottom],
        [TopRight, BottomRight, TopRight, BottomLeft],
    ];
    let mut cases = Vec::new();
    for n in 0..81 {
        let place = |k: u32| [SIDE, LAVA, OFF][(n / 3u32.pow(k) % 3) as usize];
        let places = [place(0), place(1), place(2), place(3)];
        for (p, anchors) in patterns.iter().enumerate() {
            let base = Case {
                minimal: (n + p as u32).is_multiple_of(7),
                pomodoro_chip: n.is_multiple_of(2).then_some(7),
                music_track: n % 4 != 1,
                music_cover: n % 3 != 2,
                ..with(case(blocks()), places, *anchors)
            };
            let cover_place = [OFF, LAVA, SIDE][(n as usize + p) % 3];
            cases.push(cover(base, cover_place, anchors[(n as usize) % 4]));
        }
    }
    let built: Vec<(Case, Vec<DockItem>)> = cases.iter().map(|c| (*c, c.items())).collect();
    for cols in (12..=300).step_by(9) {
        for rows in (5..=90).step_by(5) {
            for (case, items) in &built {
                let l = layout(Rect::new(0, 0, cols, rows), &case.input(items));
                check_items(&l, case, items);
            }
        }
    }
}

#[test]
fn huge_sizes_are_clean() {
    let base = case(blocks());
    for (cols, rows) in [(400, 120), (500, 40), (60, 300), (1000, 1000)] {
        for minimal in [false, true] {
            for p in [SIDE, LAVA] {
                for v in [
                    with(base, [p, p, OFF, OFF], base.anchors),
                    with(base, [p, p, p, p], base.anchors),
                ] {
                    let v = Case { minimal, ..v };
                    check(&at(cols, rows, &v), &v);
                }
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

fn chip_widgets(l: &Layout) -> Vec<usize> {
    l.chips
        .as_ref()
        .map(|c| c.items.iter().map(|i| i.widget).collect())
        .unwrap_or_default()
}

#[test]
fn micro_is_lamp_only() {
    let l = at(16, 6, &case(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 16, 6)));
    assert!(l.status.is_none() && l.panel.is_none() && l.chips.is_none());
}

#[test]
fn tiny_is_the_lamp_with_a_corner_chip() {
    let l = at(20, 8, &case(blocks()));
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 20, 8)));
    assert!(l.status.is_none() && l.panel.is_none());
    assert_eq!(l.chips.unwrap().rect, Rect::new(13, 7, 7, 1));
}

#[test]
fn small_50x16_has_status_and_chip_but_no_panel() {
    let l = at(50, 16, &case(blocks()));
    assert!(l.status.is_some());
    assert!(l.panel.is_none(), "lamp would keep only 56 % of the width");
    assert_eq!(chip_widgets(&l), [CLOCK]);
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
fn wide_short_160x22_puts_the_widgets_in_a_strip() {
    // A column beside the lamp would hold blocks L (34); the strip under
    // it has room for L with seconds (54), side by side with the
    // pomodoro, and the lamp keeps the whole width.
    let c = case(blocks());
    let l = at(160, 22, &c);
    let lamp = l.lamp.unwrap();
    let p = l.panel.as_ref().unwrap();
    assert_eq!(lamp.width, 160);
    assert_eq!(p.rect.y, lamp.bottom() + 1, "a blank row above the strip");
    let (clock, pomo) = (p.items[0].rect, p.items[1].rect);
    assert!(clock.right() < pomo.x, "side by side");
    assert!(clock.y <= pomo.y && pomo.bottom() <= clock.bottom());
    let (form, ..) = clock_of(&l, &c).unwrap();
    assert!(form.seconds, "{form:?}");
    // Not as wide-short (80×24, 120×36): the column, as before.
    assert!(at(80, 24, &c).panel.unwrap().rect.x >= 56);
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
fn a_cramped_portrait_wraps_widgets_into_rows() {
    // 70×40 is portrait: a column under the lamp would leave music only
    // its compact form; rows across the width keep it a card, beside the
    // clock and the pomodoro.
    let c = with(
        case(blocks()),
        [SIDE, SIDE, SIDE, OFF],
        case(blocks()).anchors,
    );
    let l = at(70, 40, &c);
    let p = l.panel.as_ref().unwrap();
    let rect = |w: usize| p.items.iter().find(|i| i.widget == w).unwrap().rect;
    let (pomo, music) = (rect(POMODORO), rect(2));
    assert!(pomo.right() < music.x, "side by side in one row");
    assert!(pomo.y < music.bottom() && music.y < pomo.bottom());
    assert_eq!(music.height, 6, "a card with its cover beside");
    // A column under the lamp would only have had room for less.
    assert!(at(34, 56, &c).placed(2).is_none_or(|m| m.rect.height <= 6));
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
fn a_tall_panel_on_a_huge_screen_takes_two_columns() {
    let c = cover(
        with(
            case(blocks()),
            [SIDE, SIDE, SIDE, OFF],
            case(blocks()).anchors,
        ),
        SIDE,
        Anchor::TopRight,
    );
    let l = at(250, 70, &c);
    let p = l.panel.as_ref().unwrap();
    let xs: Vec<u16> = p.items.iter().map(|i| i.rect.x).collect();
    assert!(xs[0] < xs[3], "the cover in the second column: {xs:?}");
    assert!(p.rect.height * 2 <= 69, "no taller than half the screen");
    assert_eq!(face_tier(&l, &c), Some((Tier::XL, 51, 8)));
    // Two widgets fit in one column there: they stay in one.
    let p = at(250, 70, &case(blocks())).panel.unwrap();
    assert_eq!(p.items[0].rect.x, p.items[1].rect.x);
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
    assert_eq!(l.chips.unwrap().rect, Rect::new(73, 23, 7, 1));
}

#[test]
fn minimal_off_hides_the_clock_but_not_the_pomodoro() {
    let off = Case {
        minimal: true,
        clock_chip: false,
        ..case(blocks())
    };
    assert!(at(60, 12, &off).chips.is_none());
    let pomo = Case {
        pomodoro_chip: Some(7),
        ..off
    };
    assert_eq!(chip_widgets(&at(60, 12, &pomo)), [POMODORO]);
}

#[test]
fn the_chip_row_drops_the_least_important_first() {
    let v = Case {
        minimal: true,
        pomodoro_chip: Some(7),
        ..with(
            case(blocks()),
            [SIDE, SIDE, SIDE, OFF],
            case(blocks()).anchors,
        )
    };
    // 14:32 · ▸ 18:24 · ▶ title – artist: 5 + 7 + 24 + 2 × 3 + 2 = 44.
    let l = at(80, 24, &v);
    assert_eq!(chip_widgets(&l), [CLOCK, POMODORO, 2]);
    assert_eq!(l.chips.unwrap().rect, Rect::new(36, 23, 44, 1));
    // Narrower: the clock (rank 1) goes, then music (2); the running
    // pomodoro (3) stays longest.
    assert_eq!(chip_widgets(&at(40, 14, &v)), [POMODORO, 2]);
    assert_eq!(chip_widgets(&at(30, 10, &v)), [POMODORO]);
}

#[test]
fn hidden_clock_keeps_the_pomodoro_panel() {
    let v = with(
        case(blocks()),
        [OFF, SIDE, OFF, OFF],
        case(blocks()).anchors,
    );
    let p = at(80, 24, &v).panel.unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].widget, POMODORO);
    assert_eq!(p.rect.height, 3);
}

// --- widgets on the lava --------------------------------------------------

#[test]
fn both_on_the_lava_give_the_lamp_the_whole_width() {
    let c = with(
        case(blocks()),
        [LAVA, LAVA, OFF, OFF],
        case(blocks()).anchors,
    );
    let l = at(80, 24, &c);
    assert!(l.panel.is_none() && l.chips.is_none());
    assert_eq!(l.lamp, Some(Rect::new(0, 0, 80, 23)));
    assert_eq!(l.on_lava.len(), 1, "one anchor, one stack");
    let s = &l.on_lava[0];
    // Blocks L over the full pomodoro, centred, a row apart.
    assert_eq!(face_tier(&l, &c), Some((Tier::L, 34, 5)));
    assert_eq!(s.rect, Rect::new(23, 7, 34, 9));
    assert_eq!(s.items[1].rect, Rect::new(23, 13, 34, 3));
}

#[test]
fn the_clock_on_the_lava_leaves_the_pomodoro_in_the_panel() {
    let c = with(
        case(blocks()),
        [LAVA, SIDE, OFF, OFF],
        case(blocks()).anchors,
    );
    let l = at(80, 24, &c);
    let p = l.panel.as_ref().unwrap();
    assert_eq!((p.items.len(), p.items[0].widget), (1, POMODORO));
    assert_eq!(l.on_lava[0].items[0].widget, CLOCK);
    assert!(l.lamp.unwrap().width < 80);
}

#[test]
fn the_lava_stack_shrinks_then_falls_back_to_the_chip() {
    let c = with(
        case(blocks()),
        [LAVA, SIDE, OFF, OFF],
        case(blocks()).anchors,
    );
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
    assert!(l.on_lava.is_empty());
    assert_eq!(chip_widgets(&l), [CLOCK]);
}

#[test]
fn anchors_put_the_stack_where_they_say() {
    let base = case(blocks());
    let lamp = |a| {
        let l = at(120, 36, &with(base, [LAVA, SIDE, OFF, OFF], [a; 4]));
        (l.lamp.unwrap(), l.on_lava[0].rect)
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
fn different_anchors_spread_across_the_lava() {
    use Anchor::*;
    let c = with(
        case(blocks()),
        [LAVA, LAVA, LAVA, LAVA],
        [Center, TopRight, TopLeft, Bottom],
    );
    let l = at(200, 60, &c);
    assert_eq!(l.on_lava.len(), 4, "{:?}", l.on_lava);
    assert!(l.chips.is_none(), "everything fits");
    let lamp = l.lamp.unwrap();
    let bottom = l.on_lava.iter().find(|s| s.items[0].widget == 3).unwrap();
    assert!(bottom.rect.y > lamp.height / 2, "lyrics low down");
    // A crowded lamp sheds the least important to the chips, not all.
    let small = at(90, 30, &c);
    assert!(!small.on_lava.is_empty());
    assert!(
        small.placed(POMODORO).is_some(),
        "the running pomodoro stays"
    );
}

#[test]
fn no_seconds_on_the_lava() {
    let c = with(
        case(blocks()),
        [LAVA, SIDE, OFF, OFF],
        case(blocks()).anchors,
    );
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
    assert_eq!(keys(200), "scpmSpace,?");
    // m, Space, p, c, s go first, then `, settings`; ? help last.
    let mut seen = Vec::new();
    for avail in (0..=200).rev() {
        let k = keys(avail);
        if seen.last() != Some(&k) {
            seen.push(k);
        }
    }
    assert_eq!(
        seen,
        [
            "scpmSpace,?",
            "scpSpace,?",
            "scp,?",
            "sc,?",
            "s,?",
            ",?",
            "?",
            ""
        ]
    );
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
/// `d` date, `o` pomodoro, `m` music, `y` lyrics, `~` the
/// backing of the widgets on the lava (their own cells as in the panel),
/// `S` status, `c` chip row, `t` toast row, `.` background.
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
    for s in &l.on_lava {
        fill(halo(s.rect), '~');
    }
    for p in l.panel.iter().chain(&l.on_lava) {
        for item in &p.items {
            match item.widget {
                POMODORO => fill(item.rect, 'o'),
                2 => fill(item.rect, 'm'),
                3 => fill(item.rect, 'y'),
                COVER => fill(item.rect, 'v'),
                _ => {}
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
    if let Some(c) = &l.chips {
        fill(c.rect, 'c');
    }
    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>() + "\n")
        .collect()
}

#[test]
fn snapshots_at_mockup_sizes() {
    use Anchor::*;
    let full = case(blocks());
    let minimal = Case {
        minimal: true,
        ..full
    };
    let a = full.anchors;
    let lava = with(full, [LAVA, LAVA, OFF, OFF], a);
    let all = |places| with(full, places, a);
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
            with(full, [LAVA, SIDE, OFF, OFF], a),
        ),
        (
            "lava_top_right_160x40",
            160,
            40,
            with(full, [LAVA, LAVA, OFF, OFF], [TopRight; 4]),
        ),
        (
            "lava_minimal_80x24",
            80,
            24,
            Case {
                minimal: true,
                ..with(full, [LAVA, LAVA, OFF, OFF], [BottomLeft; 4])
            },
        ),
        ("music_strip_160x22", 160, 22, all([SIDE, SIDE, SIDE, OFF])),
        (
            "music_two_columns_250x70",
            250,
            70,
            all([SIDE, SIDE, SIDE, OFF]),
        ),
        ("music_portrait_40x60", 40, 60, all([SIDE, SIDE, SIDE, OFF])),
        ("music_wrap_70x40", 70, 40, all([SIDE, SIDE, SIDE, OFF])),
        ("spread_200x60", 200, 60, all([LAVA, LAVA, LAVA, LAVA])),
        (
            "spread_corners_120x36",
            120,
            36,
            with(
                full,
                [LAVA, SIDE, LAVA, LAVA],
                [TopRight, Center, TopLeft, Bottom],
            ),
        ),
        (
            "cover_side_120x36",
            120,
            36,
            cover(all([SIDE, SIDE, SIDE, OFF]), SIDE, TopRight),
        ),
        (
            "cover_lava_160x40",
            160,
            40,
            cover(with(full, [SIDE, SIDE, LAVA, OFF], a), LAVA, TopRight),
        ),
        (
            "cover_fill_250x70",
            250,
            70,
            Case {
                cover_size: CoverSize::Fill,
                ..cover(all([SIDE, SIDE, SIDE, OFF]), SIDE, TopRight)
            },
        ),
        (
            "chips_minimal_80x24",
            80,
            24,
            Case {
                minimal: true,
                pomodoro_chip: Some(7),
                ..all([SIDE, SIDE, SIDE, OFF])
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

/// The layout runs every frame: its cost with four widgets in mixed
/// places, worst size first. `cargo test --release -- --ignored
/// --nocapture bench_layout`.
#[test]
#[ignore]
fn bench_layout() {
    use Anchor::*;
    let base = case(blocks());
    let cases = [
        ("default", base),
        ("3 side", with(base, [SIDE, SIDE, SIDE, OFF], base.anchors)),
        ("4 side", with(base, [SIDE, SIDE, SIDE, SIDE], base.anchors)),
        (
            "4 lava",
            with(
                base,
                [LAVA, LAVA, LAVA, LAVA],
                [Center, TopRight, TopLeft, Bottom],
            ),
        ),
        ("mixed", with(base, [SIDE, LAVA, SIDE, LAVA], base.anchors)),
    ];
    for (name, c) in cases {
        let items = c.items();
        let mut worst = (0.0, 0, 0);
        let mut total = 0.0;
        let mut n = 0;
        for cols in (20..=300).step_by(20) {
            for rows in (8..=90).step_by(6) {
                let t = std::time::Instant::now();
                for _ in 0..20 {
                    std::hint::black_box(layout(Rect::new(0, 0, cols, rows), &c.input(&items)));
                }
                let us = t.elapsed().as_secs_f64() * 1e6 / 20.0;
                total += us;
                n += 1;
                if us > worst.0 {
                    worst = (us, cols, rows);
                }
            }
        }
        println!(
            "{name:8} mean {:6.1} µs  worst {:6.1} µs at {}x{}",
            total / f64::from(n),
            worst.0,
            worst.1,
            worst.2
        );
    }
}
