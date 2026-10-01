//! Render tests for the chrome (lava-ebq.20): whole frames drawn by
//! [`super::draw`] into ratatui's `TestBackend`, at the mockup sizes, with
//! help, each picker, toasts, the HUD, a running pomodoro and minimal mode.
//!
//! The pictures print every cell's glyph, except that the lamp's own cells
//! (anything in the lamp view not painted on the app background) print as
//! `~`: chrome always paints `bg`, the wax never does. That keeps the
//! snapshots about the chrome, not the sim (a test checks two seeds give
//! the same picture). `UPDATE_SNAPSHOTS=1 cargo test` rewrites
//! `ui/snapshots/render_*.txt`; review the diff.

use std::fmt::Write as _;
use std::time::{Instant, SystemTime};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::chrome::{HINTS, PICKER_HINTS};
use super::keymap::{Action, InputMode, KEYMAP, action_for};
use super::picker::{self, Placement, grow};
use crate::app::{LocalTime, Model, Overlay};
use crate::clock::ClockTime;
use crate::config::store::Store;
use crate::config::{ColorChoice, Session};
use crate::render::LampState;
use crate::theme::Role;

/// The mockup sizes (§1.5) plus the micro / tiny ones from the beads.
const SIZES: &[(u16, u16)] = &[
    (12, 5),
    (20, 8),
    (30, 10),
    (50, 16),
    (72, 18),
    (80, 24),
    (120, 36),
];

fn local() -> LocalTime {
    LocalTime {
        time: ClockTime::new(14, 32, 7).unwrap(),
        date: "thu 1 oct".into(),
        wall: SystemTime::UNIX_EPOCH,
    }
}

fn model(cols: u16, rows: u16, seed: u64) -> (Model, Instant) {
    let dir = std::env::temp_dir().join(format!(
        "lavatui-render-{cols}x{rows}-{seed}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let session = Session {
        color: Some(ColorChoice::Truecolor),
        ..Session::default()
    };
    let t0 = Instant::now();
    let mut m = Model::new(
        &session,
        Store::new(Some(dir.join("config.toml"))),
        Rect::new(0, 0, cols, rows),
        None,
        local(),
        seed,
        t0,
    );
    m.stats.fps = 60.0;
    m.stats.frame_ms = 2.1;
    (m, t0)
}

fn draw(m: &Model, cols: u16, rows: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut lamp = LampState::default();
    terminal
        .draw(|frame| super::draw(frame, m, &mut lamp))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Glyphs row by row, lamp cells as `~` (see the module docs).
fn picture(m: &Model, buf: &Buffer) -> String {
    let bg = m.theme.role(Role::Bg);
    let view = m.layout.lamp.map(|l| l.view);
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let lamp = view.is_some_and(|v| v.contains((x, y).into())) && cell.bg != bg;
            line.push_str(if lamp { "~" } else { cell.symbol() });
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// One line of the buffer, as text.
fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height).map(|y| row(buf, y) + "\n").collect()
}

/// Every chrome state the tests draw: a name and how to get there.
type Setup = fn(&mut Model, Instant);

fn scenarios() -> Vec<(&'static str, Setup)> {
    vec![
        ("plain", |_, _| {}),
        ("help", |m, t| m.update(Action::Help, t)),
        ("style picker", |m, t| m.update(Action::StylePicker, t)),
        ("face picker", |m, t| m.update(Action::FacePicker, t)),
        ("palette picker", |m, t| m.update(Action::PalettePicker, t)),
        ("toast + hud", |m, t| {
            m.update(Action::DebugHud, t);
            m.toast("focus · 25:00");
        }),
        ("pomodoro running", |m, t| {
            m.update(Action::PomodoroToggle, t);
            m.toast = None;
        }),
        ("minimal + hud", |m, t| {
            m.update(Action::ToggleMinimal, t);
            m.update(Action::DebugHud, t);
            m.toast = None;
        }),
    ]
}

fn scene(cols: u16, rows: u16, seed: u64, setup: Setup) -> (Model, Buffer) {
    let (mut m, t0) = model(cols, rows, seed);
    setup(&mut m, t0);
    let buf = draw(&m, cols, rows);
    (m, buf)
}

#[test]
fn snapshots_at_mockup_sizes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/snapshots");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let mut stale = Vec::new();
    for &(cols, rows) in SIZES {
        let mut out = String::new();
        for (name, setup) in scenarios() {
            let (m, buf) = scene(cols, rows, 7, setup);
            let pic = picture(&m, &buf);
            // The pictures are about the chrome: another sim, same picture.
            let (m2, buf2) = scene(cols, rows, 99, setup);
            assert_eq!(
                pic,
                picture(&m2, &buf2),
                "{cols}x{rows} {name}: wax leaks in"
            );
            let _ = writeln!(out, "── {name} ──");
            out.push_str(&pic);
        }
        let path = dir.join(format!("render_{cols}x{rows}.txt"));
        if update {
            std::fs::write(&path, &out).unwrap();
        } else if std::fs::read_to_string(&path).ok().as_deref() != Some(out.as_str()) {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "render snapshots differ (UPDATE_SNAPSHOTS=1 to rewrite, then review): {stale:?}"
    );
}

/// lava-ebq.11: a picker sheet never slices the panel. It hides whole
/// (style, palette), or (face) the sheet moves aside so the face it
/// previews stays in view.
#[test]
fn picker_sheets_never_cut_the_panel() {
    for (cols, rows) in [(80, 24), (120, 36), (100, 30), (160, 50)] {
        for opener in [
            Action::StylePicker,
            Action::FacePicker,
            Action::PalettePicker,
        ] {
            let (mut m, t0) = model(cols, rows, 7);
            let panel = m.layout.panel.expect("panel at this size").rect;
            m.update(opener, t0);
            let buf = draw(&m, cols, rows);
            let Overlay::Picker(p) = m.overlay else {
                panic!()
            };
            let Some(Placement::Sheet { sheet, .. }) =
                picker::placement(m.layout.area, &m.layout, &p)
            else {
                panic!("{cols}x{rows}: a sheet")
            };
            let ctx = format!("{cols}x{rows} {opener:?}");
            let panel_drawn = panel
                .positions()
                .any(|pos| !sheet.contains(pos) && buf[pos].symbol() != " ");
            if grow(sheet, 1).intersects(panel) {
                assert!(
                    !panel_drawn,
                    "{ctx}: panel cut by the sheet\n{}",
                    text(&buf)
                );
            } else {
                assert!(panel_drawn, "{ctx}: panel clear of the sheet but hidden");
            }
            if opener == Action::FacePicker && cols >= 80 {
                assert!(
                    panel_drawn,
                    "{ctx}: the face picker hides the face it previews"
                );
            }
        }
    }
}

/// lava-ebq.15: a toast and the corner HUD never share a row's cells; the
/// toast wins.
#[test]
fn toast_outranks_the_corner_hud() {
    for (cols, rows) in [(30, 10), (20, 8), (40, 12), (16, 6)] {
        let (mut m, t0) = model(cols, rows, 7);
        m.update(Action::DebugHud, t0);
        m.toast("focus · 25:00");
        let buf = draw(&m, cols, rows);
        let all = text(&buf);
        assert!(all.contains("focus"), "{cols}x{rows}: toast shown\n{all}");
        let top = row(&buf, 0);
        assert!(
            !(top.contains("fps") && top.contains("focus")),
            "{cols}x{rows}: HUD and toast share the top row: {top:?}"
        );
        // No toast: the HUD is back.
        m.toast = None;
        let buf = draw(&m, cols, rows);
        if cols >= 30 {
            assert!(row(&buf, 0).contains("fps"), "{cols}x{rows}: HUD back");
        }
    }
}

/// lava-ebq.16: the inline picker always shows something at ≥ 4 cols,
/// whatever the item.
#[test]
fn inline_picker_always_shows_the_item() {
    for cols in 4..40 {
        for rows in [2, 5, 8] {
            let (mut m, t0) = model(cols, rows, 7);
            m.update(Action::StylePicker, t0);
            let Overlay::Picker(p) = m.overlay else {
                panic!()
            };
            for i in 0..p.kind.items().len() {
                m.update(Action::Jump(i as u8), t0);
                if i >= 9 {
                    m.update(Action::Down, t0);
                }
                let Overlay::Picker(p) = m.overlay else {
                    panic!()
                };
                let name = p.kind.items()[p.cursor];
                let buf = draw(&m, cols, rows);
                let top = row(&buf, 0);
                let first: String = name.chars().take(2).collect();
                assert!(
                    top.contains(&first),
                    "{cols}x{rows}: {name:?} vanished: {top:?}"
                );
                if usize::from(cols) >= name.chars().count() + 4 {
                    assert!(
                        top.contains(&format!("‹ {name} ›")),
                        "{cols}x{rows}: {top:?}"
                    );
                }
            }
        }
    }
}

/// Every keymap label shows in help at 80x24 and 120x36 (scrolling where
/// it must): help can't silently drop a row.
#[test]
fn help_shows_every_binding() {
    for (cols, rows) in [(80, 24), (120, 36), (50, 16), (30, 10)] {
        let (mut m, t0) = model(cols, rows, 7);
        m.update(Action::Help, t0);
        let max = super::help::max_scroll(m.layout.area);
        let mut seen = String::new();
        for scroll in 0..=max {
            m.overlay = Overlay::Help { scroll };
            seen.push_str(&text(&draw(&m, cols, rows)));
        }
        for r in KEYMAP {
            // Roomy sizes show labels whole; narrow ones at least their
            // first word.
            let label = if cols >= 80 {
                r.label
            } else {
                r.label.split([' ', ':']).next().unwrap()
            };
            assert!(
                seen.contains(&format!("{}  {label}", r.keys)) || seen.contains(label),
                "{cols}x{rows}: help lacks {:?}\n{seen}",
                r.label
            );
        }
    }
}

/// §4.1: `● style · palette` on the left (palette from 60 cols), hints on
/// the right, `? help` last to go, at least 4 cols between.
#[test]
fn status_bar_matches_the_spec() {
    for (cols, rows) in [(50, 16), (72, 18), (80, 24)] {
        let (m, _) = model(cols, rows, 7);
        let buf = draw(&m, cols, rows);
        let status = m.layout.status.expect("status bar");
        let line = row(&buf, status.y);
        let name = m.style.style().name();
        let left = line.trim_start();
        assert!(left.starts_with(&format!("● {name}")), "{line:?}");
        assert_eq!(left.contains("· lava"), cols >= 60, "{line:?}");
        assert!(line.trim_end().ends_with("? help"), "{line:?}");
        let gap = line.trim().split("    ").count();
        assert!(gap >= 2, "a 4-col gap between left and hints: {line:?}");
        // Nothing outside the inset.
        let inset = usize::from(status.x);
        assert!(line.chars().take(inset).all(|c| c == ' '), "{line:?}");
        assert!(line.chars().rev().take(inset).all(|c| c == ' '), "{line:?}");
    }
    // 80x24 (§4.1 mockup): the first hints and `? help`, in display order.
    let (m, _) = model(80, 24, 7);
    let buf = draw(&m, 80, 24);
    let line = row(&buf, m.layout.status.unwrap().y);
    let at = |h: &str| line.find(h).unwrap_or_else(|| panic!("{h:?} in {line:?}"));
    assert!(at("s style") < at("c clock") && at("c clock") < at("p palette"));
    assert!(at("p palette") < at("␣ pomo") && at("␣ pomo") < at("? help"));
}

fn key(k: &str) -> Event {
    let code = match k {
        "␣" => KeyCode::Char(' '),
        "⏎" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "↑↓" => KeyCode::Up,
        k => KeyCode::Char(k.chars().next().unwrap()),
    };
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// The status bar's hints are hard-coded text: each must still be the key
/// the keymap binds to that action.
#[test]
fn hint_keys_resolve_through_the_keymap() {
    let expect = [
        ("s", Action::NextStyle),
        ("c", Action::NextFace),
        ("p", Action::NextPalette),
        ("f", Action::CycleFrame),
        ("l", Action::ToggleLighting),
        ("m", Action::ToggleMinimal),
        ("␣", Action::PomodoroToggle),
        ("?", Action::Help),
    ];
    assert_eq!(HINTS.len(), expect.len());
    for (k, label, _) in HINTS {
        let want = expect.iter().find(|(e, _)| e == k).expect("known hint").1;
        assert_eq!(
            action_for(&key(k), InputMode::Normal),
            Some(want),
            "{k} {label}"
        );
    }
    let picker = InputMode::Picker {
        opener: Action::StylePicker,
        inline: false,
    };
    for (k, label, _) in PICKER_HINTS {
        let want = match *label {
            "preview" => Action::Up,
            "keep" => Action::Keep,
            "revert" => Action::Close,
            other => panic!("unknown picker hint {other}"),
        };
        assert_eq!(action_for(&key(k), picker), Some(want), "{k} {label}");
    }
}

/// Every size × chrome state draws without writing outside the buffer
/// (TestBackend panics on an out-of-range index), including frames drawn
/// at a size the model wasn't laid out for (mid-resize).
#[test]
fn every_state_draws_at_every_size() {
    for cols in (1..=130).step_by(7) {
        for rows in (1..=45).step_by(4) {
            for (name, setup) in scenarios() {
                let (m, buf) = scene(cols, rows, 7, setup);
                assert_eq!(buf.area, Rect::new(0, 0, cols, rows), "{name}");
                // Drawn at a different size than laid out.
                let _ = draw(&m, cols / 2 + 1, rows + 3);
            }
        }
    }
}

/// lava-ebq.3, §9: `theme.transparent` leaves everything outside the glass
/// to the terminal, the bottle rect's corners included.
#[test]
fn transparent_never_paints_bg() {
    for (cols, rows) in [(80, 24), (120, 36), (34, 56), (50, 16)] {
        let (mut m, _) = model(cols, rows, 7);
        let bg = m.theme.role(Role::Bg);
        assert!(
            draw(&m, cols, rows).content().iter().any(|c| c.bg == bg),
            "{cols}x{rows}: bg is painted normally"
        );
        m.settings.theme.transparent = true;
        let buf = draw(&m, cols, rows);
        // The chip is chrome on the lamp: its pad is meant to be solid.
        let chip = m.layout.chip.map(|c| c.rect).unwrap_or_default();
        let painted: Vec<_> = buf
            .area
            .positions()
            .filter(|&p| buf[p].bg == bg && !chip.contains(p))
            .collect();
        assert!(painted.is_empty(), "{cols}x{rows}: bg at {painted:?}");
    }
}

/// lava-ebq.3, §7: a reduced grid still draws the whole lamp (just from
/// fewer samples), and the HUD counts the samples actually taken.
#[test]
fn reduced_grid_draws_the_whole_lamp() {
    let (mut m, t0) = model(80, 24, 7);
    m.update(Action::DebugHud, t0);
    m.update(Action::ToggleMinimal, t0);
    m.toast = None;
    let full = draw(&m, 80, 24);
    let dt = std::time::Duration::from_millis(16);
    let mut t = t0;
    while !m.quality.reduced_grid() {
        t += dt;
        m.frame_drawn(30.0, dt, t);
    }
    let reduced = draw(&m, 80, 24);
    // Every lamp cell is still painted (bottle colours, never left blank).
    let view = m.layout.lamp.unwrap().view;
    let reset = ratatui::style::Color::Reset;
    assert!(view.positions().all(|p| reduced[p].bg != reset));
    let hud = |b: &Buffer| row(b, 0);
    assert_ne!(hud(&full), hud(&reduced), "HUD shows fewer samples");
    let px = |s: String| -> usize {
        let k = s.split(" · ").nth(2).unwrap().trim();
        k.trim_end_matches("k px").parse().unwrap()
    };
    assert!(px(hud(&reduced)) * 3 < px(hud(&full)) + 3);
}
