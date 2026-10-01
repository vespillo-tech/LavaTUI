//! Render tests for the chrome (lava-ebq.20): whole frames drawn by
//! [`super::draw`] into ratatui's `TestBackend`, at the mockup sizes, with
//! help, each picker, toasts, the HUD, a running pomodoro and minimal mode.
//!
//! The pictures print every cell's glyph, except that the lamp's own cells
//! (anything in the lamp view not painted on the app background, outside
//! the widgets on the lava) print as `~`: chrome always paints `bg`, the
//! wax never does. That keeps the
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

use super::chrome::{HINTS, PICKER_HINTS, PLAYER_HINTS};
use super::keymap::{Action, InputMode, KEYMAP, PlayerKey, action_for};
use super::picker::{self, Placement, grow};
use crate::app::{LocalTime, Model, Overlay};
use crate::clock::ClockTime;
use crate::config::store::Store;
use crate::config::{ColorChoice, Session};
use crate::media::art::{Art, ArtLoader};
use crate::media::{FakeSource, Snapshot, Status, Track, Unavailable};
use crate::render::LampState;
use crate::theme::{Rgb, Role};

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
    let view = m.layout.lamp;
    // The widgets on the lava sit on a cleared backing: print them.
    let lava = m.layout.on_lava.as_ref().map(|s| s.rect);
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let pos = (x, y).into();
            let lamp = view.is_some_and(|v| v.contains(pos))
                && lava.is_none_or(|l| !l.contains(pos))
                && cell.bg != bg;
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
        ("minimal pomodoro focus", |m, t| {
            m.update(Action::ToggleMinimal, t);
            m.update(Action::PomodoroToggle, t);
            m.toast = None;
        }),
        ("minimal pomodoro break", |m, t| {
            m.update(Action::ToggleMinimal, t);
            m.update(Action::PomodoroToggle, t);
            m.update(Action::PomodoroSkip, t);
            m.toast = None;
            m.flash = None;
        }),
        ("clock on the lava", |m, t| {
            m.update(Action::Place("clock"), t);
            m.toast = None;
        }),
        ("both on the lava, top left, running", |m, t| {
            m.update(Action::Place("clock"), t);
            m.update(Action::Place("pomodoro"), t);
            for _ in 0..6 {
                m.update(Action::NextAnchor, t);
            }
            m.update(Action::PomodoroToggle, t);
            m.toast = None;
        }),
        ("minimal + hud", |m, t| {
            m.update(Action::ToggleMinimal, t);
            m.update(Action::DebugHud, t);
            m.toast = None;
        }),
        ("music beside, player keys", |m, t| {
            music(m, t, Status::Playing, 1);
            m.update(Action::PlayerKeys, t);
            m.toast = None;
        }),
        ("music on the lava, paused", |m, t| {
            m.update(Action::Place("pomodoro"), t);
            music(m, t, Status::Paused, 2);
        }),
        ("music: spotify not running", |m, t| {
            music(m, t, Status::Unavailable(Unavailable::NotRunning), 1);
        }),
        ("minimal music chip", |m, t| {
            music(m, t, Status::Playing, 1);
            m.update(Action::ToggleMinimal, t);
            m.toast = None;
        }),
    ]
}

const COVER: &str = "https://i.example/cover";

/// The music widget on a fake player in `status`, placed by `presses` of
/// `a` (1 side, 2 on the lava), its cover already loaded.
fn music(m: &mut Model, t: Instant, status: Status, presses: usize) {
    let track = Track {
        id: "fake:1".into(),
        name: "Convection (Long Version)".into(),
        artist: "Wax & Wane".into(),
        album: "Lamplight".into(),
        duration: std::time::Duration::from_secs(402),
        artwork_url: COVER.into(),
    };
    let fake = FakeSource::new(
        Snapshot {
            player: Some("Spotify".into()),
            track: Some(std::sync::Arc::new(track)),
            position: std::time::Duration::from_secs(97),
            volume: 70,
            ..Snapshot::new(status, t)
        },
        Vec::new(),
    );
    m.music.connect_with(
        move || Box::new(fake.clone()),
        || ArtLoader::preloaded(COVER, Art::solid(Rgb(200, 120, 40))),
    );
    for _ in 0..presses {
        m.update(Action::Place("music"), t);
    }
    m.toast = None;
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
            let panel = m.layout.panel.as_ref().expect("panel at this size").rect;
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
        let max = super::help::sheet::max_scroll(m.layout.area);
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
                r.label.split(' ').next().unwrap()
            };
            assert!(
                seen.contains(&format!("{}  {label}", r.keys)) || seen.contains(label),
                "{cols}x{rows}: help lacks {:?}\n{seen}",
                r.label
            );
        }
    }
}

/// lava-ebq.38: the small full-screen help leads with `m ? q`, and when
/// keys are cut off the top row says which way they scroll.
#[test]
fn small_help_pins_app_keys_and_hints_scrolling() {
    for (cols, rows) in [(20, 8), (30, 10), (40, 14), (50, 16), (60, 20)] {
        let (mut m, t0) = model(cols, rows, 7);
        m.update(Action::Help, t0);
        let buf = draw(&m, cols, rows);
        let ctx = format!("{cols}x{rows}");
        for (y, key) in [(3, "m"), (4, "?"), (5, "q")] {
            assert!(row(&buf, y).trim_start().starts_with(key), "{ctx}: {key}");
        }
        let max = super::help::sheet::max_scroll(m.layout.area);
        assert!(max > 0, "{ctx}: the keys don't all fit");
        let top = |m: &mut Model, scroll| {
            m.overlay = Overlay::Help { scroll };
            row(&draw(m, cols, rows), 0)
        };
        let first = top(&mut m, 0);
        assert!(
            first.contains('↓') && first.contains("esc close"),
            "{ctx}: {first:?}"
        );
        assert!(top(&mut m, max).contains('↑'), "{ctx}");
        if max > 1 {
            assert!(top(&mut m, 1).contains('↕'), "{ctx}");
        }
    }
    // All of it fits: no hint.
    let (mut m, t0) = model(80, 24, 7);
    m.update(Action::Help, t0);
    let buf = text(&draw(&m, 80, 24));
    // The hint sits in the title row (the music keys show arrows too).
    let title = buf.lines().find(|l| l.contains("╭ keys")).unwrap();
    assert!(!title.contains('↓') && !title.contains('↕'), "{buf}");
}

/// lava-ebq.43: the two-column sheet keeps a 2-col gutter, and every
/// label in a column starts at one x.
#[test]
fn help_sheet_columns_have_a_gutter_and_aligned_labels() {
    for (cols, rows) in [
        (68, 20),
        (80, 24),
        (100, 30),
        (160, 22),
        (200, 50),
        (300, 100),
    ] {
        let (mut m, t0) = model(cols, rows, 7);
        m.update(Action::Help, t0);
        let buf = draw(&m, cols, rows);
        let Some((lines, inner)) = super::help::sheet::body(m.layout.area) else {
            panic!()
        };
        let spans = super::help::sheet::column_spans(&lines, inner);
        assert_eq!(spans.len(), 2, "{cols}x{rows}: two columns");
        let (lx, lw) = spans[0];
        let rx = spans[1].0;
        assert!(rx >= lx + lw + super::help::sheet::GUTTER, "{cols}x{rows}");
        for y in inner.top()..inner.bottom() {
            for x in lx + lw..rx {
                assert_eq!(
                    buf[(x, y)].symbol(),
                    " ",
                    "{cols}x{rows}: gutter at {x},{y}"
                );
            }
        }
        for (c, &(x0, w)) in spans.iter().enumerate() {
            // Where each key line's label (in `text`) starts.
            let (key, label) = (m.theme.role(Role::Accent), m.theme.role(Role::Text));
            let starts: Vec<u16> = (inner.top()..inner.bottom())
                .filter(|&y| buf[(x0, y)].fg == key)
                .filter_map(|y| (x0..x0 + w).find(|&x| buf[(x, y)].fg == label))
                .collect();
            assert!(starts.len() > 5, "{cols}x{rows}: column {c}");
            assert!(
                starts.iter().all(|&x| x == starts[0]),
                "{cols}x{rows}: column {c} labels at {starts:?}"
            );
        }
    }
}

/// lava-ebq.41: in minimal mode a focus chip and a break chip never look
/// the same, at any size that shows one.
#[test]
fn minimal_chip_tells_focus_from_break() {
    let setups = scenarios();
    let find = |name| setups.iter().find(|(n, _)| *n == name).unwrap().1;
    let (focus, rest) = (
        find("minimal pomodoro focus"),
        find("minimal pomodoro break"),
    );
    let mut seen = 0;
    for cols in (12..=300).step_by(12) {
        for rows in (5..=90).step_by(5) {
            let (mf, bf) = scene(cols, rows, 7, focus);
            let (mb, bb) = scene(cols, rows, 7, rest);
            assert_eq!(mf.layout.chip.is_some(), mb.layout.chip.is_some());
            if mf.layout.chip.is_none() {
                continue;
            }
            seen += 1;
            let (tf, tb) = (text(&bf), text(&bb));
            assert!(
                tf.contains("▸ 25:00") && !tf.contains("break"),
                "{cols}x{rows}"
            );
            assert!(tb.contains("▸ break 5:00"), "{cols}x{rows}\n{tb}");
        }
    }
    assert!(seen > 100, "chips seen: {seen}");
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
        "←→" => KeyCode::Right,
        "n p" => KeyCode::Char('n'),
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
    for (k, label, _) in PLAYER_HINTS {
        let want = match *label {
            "play" => Action::Player(PlayerKey::PlayPause),
            "skip" => Action::Player(PlayerKey::Next),
            "seek" => Action::Player(PlayerKey::SeekForward),
            "volume" => Action::Player(PlayerKey::VolumeUp),
            "done" => Action::Close,
            other => panic!("unknown player hint {other}"),
        };
        assert_eq!(
            action_for(&key(k), InputMode::Player),
            Some(want),
            "{k} {label}"
        );
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

/// lava-ebq.3, lava-ebq.21, §9: `theme.transparent` leaves the background
/// to the terminal: no chrome (panel, chip, toast, HUD, help, pickers)
/// paints `bg`.
#[test]
fn transparent_never_paints_bg() {
    for (cols, rows) in [(80, 24), (120, 36), (34, 56), (50, 16), (20, 8)] {
        for (name, setup) in scenarios() {
            let (mut m, t0) = model(cols, rows, 7);
            setup(&mut m, t0);
            let bg = m.theme.role(Role::Bg);
            assert!(
                draw(&m, cols, rows).content().iter().any(|c| c.bg == bg),
                "{cols}x{rows} {name}: bg is painted normally"
            );
            m.settings.theme.transparent = true;
            let buf = draw(&m, cols, rows);
            let painted: Vec<_> = buf.area.positions().filter(|&p| buf[p].bg == bg).collect();
            assert!(
                painted.is_empty(),
                "{cols}x{rows} {name}: bg at {painted:?}"
            );
        }
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
    // Every lamp cell is still painted (never left blank).
    let view = m.layout.lamp.unwrap();
    let reset = ratatui::style::Color::Reset;
    assert!(view.positions().all(|p| reduced[p].bg != reset));
    let hud = |b: &Buffer| row(b, 0);
    assert_ne!(hud(&full), hud(&reduced), "HUD shows fewer samples");
    let px = |s: String| -> usize {
        // `1k px`, then the lamp the HUD sits on.
        let k = s.split(" · ").nth(2).unwrap().split(' ').next().unwrap();
        k.trim_end_matches('k').parse().unwrap()
    };
    assert!(px(hud(&reduced)) * 3 < px(hud(&full)) + 3);
}
