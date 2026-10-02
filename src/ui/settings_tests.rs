//! The settings screen drawn (lava-1xk.17): every page at the sizes the
//! bead asks for (snapshots `ui/snapshots/settings_*.txt`, lamp cells as
//! `~`, `UPDATE_SNAPSHOTS=1 cargo test` rewrites them), a size sweep, and
//! clicks landing on what is drawn.

use std::fmt::Write as _;
use std::time::Instant;

use ratatui::layout::Rect;

use super::keymap::Action;
use super::render_tests::{draw, model, picture, row};
use super::settings::{self, Hit};
use crate::app::{Model, Overlay, Page};
use crate::spotify_web::fake::demo;

/// The sizes snapshotted: the usual window and a small one, plus a big
/// and a tiny one.
const SIZES: &[(u16, u16)] = &[(80, 24), (40, 14), (120, 36), (24, 9)];

type Setup = fn(&mut Model, Instant);

/// Into page `page` (by its place in the list) of the screen.
fn page(m: &mut Model, t: Instant, page: usize) {
    m.update(Action::Settings, t);
    for _ in 0..page {
        m.update(Action::Down, t);
    }
    m.update(Action::Keep, t);
}

/// Into the Spotify setup.
fn setup(m: &mut Model, t: Instant) {
    page(m, t, 3);
    m.update(Action::Keep, t);
}

fn scenarios() -> Vec<(&'static str, Setup)> {
    vec![
        ("the list of pages", |m, t| m.update(Action::Settings, t)),
        ("look", |m, t| page(m, t, 0)),
        ("look, on colours", |m, t| {
            page(m, t, 0);
            m.update(Action::Down, t);
        }),
        ("clock & timer", |m, t| page(m, t, 1)),
        ("widgets, clock on the lamp", |m, t| {
            page(m, t, 2);
            m.update(Action::Change(true), t);
            m.toast = None;
        }),
        ("music & lyrics", |m, t| page(m, t, 3)),
        ("controls", |m, t| page(m, t, 4)),
        ("window", |m, t| page(m, t, 5)),
        ("spotify setup", setup),
        ("spotify setup, a bad client id", |m, t| {
            setup(m, t);
            m.paste("my-client-secret", t);
        }),
        ("spotify setup, connected", |m, t| {
            let fake = demo();
            m.library.connect_with(move || {
                Some(Box::new(fake.clone()) as Box<dyn crate::spotify_web::Web>)
            });
            m.settings.spotify.client_id = "0123456789abcdef0123456789abcdef".into();
            setup(m, t);
            m.update(Action::Resize, t);
            m.update(Action::Resize, t);
            m.toast = None;
        }),
    ]
}

fn scene(cols: u16, rows: u16, setup: Setup) -> (Model, ratatui::buffer::Buffer) {
    let (mut m, t0) = model(cols, rows, 7);
    setup(&mut m, t0);
    let buf = draw(&m, cols, rows);
    (m, buf)
}

#[test]
fn snapshots_of_every_page() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/snapshots");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let mut stale = Vec::new();
    for &(cols, rows) in SIZES {
        let mut out = String::new();
        for (name, setup) in scenarios() {
            let (m, buf) = scene(cols, rows, setup);
            let _ = writeln!(out, "── {name} ──");
            out.push_str(&picture(&m, &buf));
        }
        let path = dir.join(format!("settings_{cols}x{rows}.txt"));
        if update {
            std::fs::write(&path, &out).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(out.as_str()) {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "settings snapshots differ (UPDATE_SNAPSHOTS=1 to rewrite, then review): {stale:?}"
    );
}

/// Every page draws at every size, inside the screen, also mid-resize.
#[test]
fn every_page_draws_at_every_size() {
    for cols in (1..=130).step_by(9) {
        for rows in (1..=45).step_by(5) {
            for (name, setup) in scenarios() {
                let (m, buf) = scene(cols, rows, setup);
                assert_eq!(buf.area, Rect::new(0, 0, cols, rows), "{name}");
                if let Overlay::Settings(view) = m.overlay {
                    let g = settings::geometry(buf.area, &view);
                    for r in g.iter().flat_map(|g| {
                        [g.pages, g.header, g.rows, g.about, g.back]
                            .into_iter()
                            .flatten()
                            .chain([g.footprint])
                    }) {
                        assert!(buf.area.contains(r.as_position()), "{cols}x{rows} {name}");
                        assert!(
                            r.right() <= cols && r.bottom() <= rows,
                            "{cols}x{rows} {name}"
                        );
                    }
                }
                let _ = draw(&m, cols / 2 + 1, rows + 3);
            }
        }
    }
}

/// A click on a drawn row lands on that row, and a click on a page in the
/// list picks that page.
#[test]
fn clicks_land_on_what_is_drawn() {
    for (cols, rows) in [(80, 24), (40, 14)] {
        let (mut m, t0) = model(cols, rows, 7);
        page(&mut m, t0, 1);
        let view = m.settings_view().unwrap();
        let rows_shown = m.settings_rows(view.page);
        let g = settings::geometry(m.layout.area, &view).unwrap();
        let r = g.rows.unwrap();
        let buf = draw(&m, cols, rows);
        for (i, want) in rows_shown.iter().enumerate().take(usize::from(r.height)) {
            let y = r.y + i as u16;
            assert!(
                row(&buf, y).contains(&want.label),
                "{cols}x{rows}: row {i} isn't {}",
                want.label
            );
            assert_eq!(
                settings::hit(m.layout.area, &m, &view, r.x + 2, y),
                Some(Hit::Row(i))
            );
        }
        // Click the focus length row: the cursor goes there; again: it changes.
        let y = r.y + 3;
        m.update(
            Action::Click {
                col: r.x + 2,
                row: y,
            },
            t0,
        );
        assert_eq!(m.settings_view().unwrap().cursor, 3);
        let before = m.settings.pomodoro.focus_min;
        m.update(
            Action::Click {
                col: r.x + 2,
                row: y,
            },
            t0,
        );
        assert_ne!(m.settings.pomodoro.focus_min, before, "{cols}x{rows}");
    }
    let (mut m, t0) = model(80, 24, 7);
    m.update(Action::Settings, t0);
    let view = m.settings_view().unwrap();
    let pages = settings::geometry(m.layout.area, &view)
        .unwrap()
        .pages
        .unwrap();
    m.update(
        Action::Click {
            col: pages.x + 3,
            row: pages.y + 4,
        },
        t0,
    );
    assert_eq!(m.settings_view().unwrap().page, Page::Controls);
}

/// The status bar names the keys that work in the screen.
#[test]
fn hints_follow_the_screen() {
    let (mut m, t0) = model(80, 24, 7);
    page(&mut m, t0, 0);
    let buf = draw(&m, 80, 24);
    let status = row(&buf, 23);
    assert!(status.contains("←→ change"), "{status}");
    m.update(Action::Close, t0);
    setup(&mut m, t0);
    m.paste("abc", t0);
    let status = row(&draw(&m, 80, 24), 23);
    assert!(status.contains("⏎ save"), "{status}");
}
