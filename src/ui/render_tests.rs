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
use super::layout::{Stack, halo};
use super::picker::{self, Placement, grow};
use crate::app::{LocalTime, Model, Overlay};
use crate::clock::ClockTime;
use crate::config::store::Store;
use crate::config::{ColorChoice, Session};
use crate::dock::{Backdrop, Look, WIDGETS};
use crate::media::art::{Art, ArtLoader};
use crate::media::{FakeSource, Snapshot, Status, Track, Unavailable};
use crate::render::LampState;
use crate::spotify_web::fake::demo;
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

pub(super) fn model(cols: u16, rows: u16, seed: u64) -> (Model, Instant) {
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
    // The welcome card has its own tests; the rest show the lamp as used.
    m.welcome = false;
    (m, t0)
}

pub(super) fn draw(m: &Model, cols: u16, rows: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut lamp = LampState::default();
    terminal
        .draw(|frame| super::draw(frame, m, &mut lamp))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Glyphs row by row, lamp cells as `~` (see the module docs).
pub(super) fn picture(m: &Model, buf: &Buffer) -> String {
    let bg = m.theme.role(Role::Bg);
    let view = m.layout.lamp;
    // The widgets on the lava float on the lamp: print what they draw
    // (spaces show the lamp, `~`), not the wax around their strokes.
    let mut lava = Buffer::empty(buf.area);
    // Left out whole under an overlay, as `ui::draw` does.
    let covered = super::overlay_footprint(buf.area, &m.layout, m);
    let shown = |s: &&Stack| covered.is_none_or(|c| !grow(c, 1).intersects(halo(s.rect)));
    let shown: Vec<&Stack> = m.layout.on_lava.iter().filter(shown).collect();
    for s in &shown {
        let look = Look {
            backdrop: Backdrop::Lava,
            align: s.align,
        };
        for p in &s.items {
            WIDGETS[p.widget].draw(m, p.form, p.rect, look, &mut lava);
        }
    }
    let stacks: Vec<Rect> = shown.iter().map(|s| s.rect).collect();
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let pos = (x, y).into();
            let on_lava = stacks.iter().any(|l| l.contains(pos));
            let lamp = view.is_some_and(|v| v.contains(pos)) && cell.bg != bg;
            let symbol = if on_lava && lava[pos].symbol() != " " {
                lava[pos].symbol()
            } else if lamp {
                "~"
            } else {
                cell.symbol()
            };
            line.push_str(symbol);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// One line of the buffer, as text.
pub(super) fn row(buf: &Buffer, y: u16) -> String {
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
        ("welcome (first start)", |m, _| m.welcome = true),
        ("face picker, clock off: a preview", |m, t| {
            m.update(Action::Place("clock"), t);
            m.update(Action::Place("clock"), t);
            m.update(Action::FacePicker, t);
            m.update(Action::Down, t);
            m.toast = None;
        }),
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
        ("both on the lava, pomodoro top left, running", |m, t| {
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
        ("lamp only, music controls after the toast", |m, t| {
            music(m, t, Status::Playing, 1);
            m.update(Action::ToggleMinimal, t);
            m.update(Action::PlayerKeys, t);
            m.toast = None;
        }),
        ("lamp only, music controls, not allowed", |m, t| {
            music(m, t, Status::Unavailable(Unavailable::PermissionDenied), 1);
            m.update(Action::ToggleMinimal, t);
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
        ("music: liked, logged in", |m, t| {
            spotify(m, t, true);
            m.toast = None;
        }),
        ("music: logged out of spotify", |m, t| {
            spotify(m, t, false);
            m.toast = None;
        }),
        ("library: playlists", |m, t| {
            spotify(m, t, true);
            m.update(Action::PlayerKeys, t);
            m.update(Action::Player(PlayerKey::Playlists), t);
            m.update(Action::Down, t);
            m.toast = None;
        }),
        ("library: a playlist's tracks", |m, t| {
            spotify(m, t, true);
            m.update(Action::PlayerKeys, t);
            m.update(Action::Player(PlayerKey::Playlists), t);
            m.update(Action::Keep, t);
            m.update(Action::Down, t);
            m.toast = None;
        }),
        ("library: filtering", |m, t| {
            spotify(m, t, true);
            m.update(Action::PlayerKeys, t);
            m.update(Action::Player(PlayerKey::Playlists), t);
            m.update(Action::Find, t);
            for c in "LA".chars() {
                m.update(Action::Type(c), t);
            }
            m.update(Action::Down, t);
            m.toast = None;
        }),
        ("library: add to playlist", |m, t| {
            spotify(m, t, true);
            m.update(Action::PlayerKeys, t);
            m.update(Action::Player(PlayerKey::AddToPlaylist), t);
            m.toast = None;
        }),
        ("library: logged out", |m, t| {
            spotify(m, t, false);
            m.update(Action::PlayerKeys, t);
            m.update(Action::Player(PlayerKey::Playlists), t);
            m.toast = None;
        }),
        ("lyrics beside, a long line wrapped", |m, t| {
            lyrics(m, t, 1, 23)
        }),
        ("lyrics on the lava, bottom", |m, t| lyrics(m, t, 2, 15)),
        ("lyrics on the lava in a gap", |m, t| lyrics(m, t, 2, 34)),
    ]
}

/// Music beside the lamp playing a Spotify track (liked), with the demo
/// account logged in (or out).
fn spotify(m: &mut Model, t: Instant, logged_in: bool) {
    let fake = demo();
    {
        let mut s = fake.state();
        s.logged_in = logged_in;
        s.liked.insert("spotify:track:t0".into());
    }
    m.library
        .connect_with(move || Some(Box::new(fake.clone()) as Box<dyn crate::spotify_web::Web>));
    music_track(m, t, Status::Playing, 1, "spotify:track:t0");
    // The account's answers land on the next frame.
    m.update(Action::Resize, t);
}

const LRC: &str = "[00:05.00]Wax rises slowly through the amber light\\n\
    [00:10.00]Cooling at the top it drifts\\n[00:14.00]And falls\\n\
    [00:20.00]Every blob that ever broke away comes home again to the warm pool below\\n\
    [00:30.00]\\n[00:40.00]Slow rise";

/// The lyrics widget on a fake player `secs` into the song, its lines
/// already fetched (from a mock LRCLIB), placed by `presses` of `y`.
fn lyrics(m: &mut Model, t: Instant, presses: usize, secs: u64) {
    use crate::lyrics::LyricsService;
    use crate::lyrics::client::Lrclib;
    use crate::lyrics::client::tests::{Mock, ok};
    let track = Track {
        id: "fake:1".into(),
        uri: None,
        name: "Slow Rise".into(),
        artist: "The Paraffins".into(),
        album: "Heat Rises".into(),
        duration: std::time::Duration::from_secs(214),
        artwork_url: String::new(),
    };
    let fake = FakeSource::new(
        Snapshot {
            player: Some("Spotify".into()),
            track: Some(std::sync::Arc::new(track)),
            position: std::time::Duration::from_secs(secs),
            ..Snapshot::new(Status::Playing, t)
        },
        Vec::new(),
    );
    m.music.connect_with(
        move || Box::new(fake.clone()),
        || ArtLoader::preloaded(COVER, Art::solid(Rgb(200, 120, 40))),
    );
    let body = format!(
        r#"{{"trackName":"Slow Rise","artistName":"The Paraffins","duration":214.0,"instrumental":false,"syncedLyrics":"{LRC}"}}"#
    );
    m.lyrics.start_with(move || {
        let mock = Mock::new([ok(&body)]);
        LyricsService::spawn(Lrclib::with_http(mock, "http://test"), None, Vec::new()).ok()
    });
    for _ in 0..presses {
        m.update(Action::Place("lyrics"), t);
    }
    let area = m.layout.area;
    for _ in 0..1000 {
        m.tick(t, area, local());
        if m.lyrics.cursor.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(m.lyrics.cursor.is_some(), "lyrics never arrived");
    m.toast = None;
}

const COVER: &str = "https://i.example/cover";

/// The music widget on a fake player in `status`, placed by `presses` of
/// `a` (1 side, 2 on the lava), its cover already loaded.
fn music(m: &mut Model, t: Instant, status: Status, presses: usize) {
    music_track(m, t, status, presses, "fake:1");
}

fn music_track(m: &mut Model, t: Instant, status: Status, presses: usize, id: &str) {
    let track = Track {
        id: id.into(),
        uri: crate::media::spotify_track_uri(id),
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
            // The sheet shows rows whole; the narrow help one action a
            // line, each label whole.
            let lines = if cols >= 80 {
                vec![(r.keys, r.label)]
            } else {
                r.split()
            };
            for (keys, label) in lines {
                assert!(
                    seen.contains(keys) && seen.contains(label),
                    "{cols}x{rows}: help lacks {keys} {label:?}\n{seen}"
                );
            }
        }
        assert!(!seen.contains(super::help::sheet::WIDEN), "{cols}x{rows}");
    }
}

/// lava-1xk.7: the narrow help never cuts a label short (a cut combined
/// row could name one action for two keys): each shows whole or not at
/// all, and when some don't, the last line says to widen the window.
#[test]
fn narrow_help_shows_whole_labels_or_says_to_widen() {
    for (cols, rows) in [(20, 8), (24, 10), (30, 10), (40, 14)] {
        let (mut m, t0) = model(cols, rows, 7);
        m.update(Action::Help, t0);
        let max = super::help::sheet::max_scroll(m.layout.area);
        let mut seen = String::new();
        for scroll in 0..=max {
            m.overlay = Overlay::Help { scroll };
            seen.push_str(&text(&draw(&m, cols, rows)));
        }
        let mut missing = false;
        for (keys, label) in KEYMAP.iter().flat_map(|r| r.split()) {
            let line = seen.lines().find(|l| {
                l.trim_start().starts_with(keys)
                    && l[l.find(keys).unwrap() + keys.len()..]
                        .trim_start()
                        .starts_with(label.split(' ').next().unwrap())
            });
            match line {
                Some(l) => assert!(l.contains(label), "{cols}x{rows}: {keys} cut: {l:?}"),
                None => missing = true,
            }
        }
        assert_eq!(
            seen.contains(super::help::sheet::WIDEN),
            missing,
            "{cols}x{rows}\n{seen}"
        );
    }
}

/// lava-1xk.13: help names the mouse's selection modifier the terminal
/// uses: shift, or option in macOS Terminal and iTerm2.
#[test]
fn help_names_the_terminals_selection_modifier() {
    for (cols, rows) in [(80, 24), (40, 14)] {
        for option in [false, true] {
            let (mut m, t0) = model(cols, rows, 7);
            m.option_drag = option;
            m.update(Action::Help, t0);
            let max = super::help::sheet::max_scroll(m.layout.area);
            let mut seen = String::new();
            for scroll in 0..=max {
                m.overlay = Overlay::Help { scroll };
                seen.push_str(&text(&draw(&m, cols, rows)));
            }
            let (yes, no) = if option {
                ("⌥ drag", "⇧ drag")
            } else {
                ("⇧ drag", "⌥ drag")
            };
            assert!(
                seen.contains(yes) && !seen.contains(no),
                "{cols}x{rows} {option}"
            );
        }
    }
}

/// lava-ebq.38: the small full-screen help leads with `m ? , q`, and when
/// keys are cut off the top row says which way they scroll.
#[test]
fn small_help_pins_app_keys_and_hints_scrolling() {
    for (cols, rows) in [(20, 8), (30, 10), (40, 14), (50, 16), (60, 20)] {
        let (mut m, t0) = model(cols, rows, 7);
        m.update(Action::Help, t0);
        let buf = draw(&m, cols, rows);
        let ctx = format!("{cols}x{rows}");
        for (y, key) in [(3, "m"), (4, "?"), (5, ","), (6, "q")] {
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
            first.contains('↓') && first.contains("Esc close"),
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
            assert_eq!(mf.layout.chips.is_some(), mb.layout.chips.is_some());
            if mf.layout.chips.is_none() {
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
    assert!(at("s style") < at("c clock") && at("c clock") < at("p colours"));
    assert!(at("p colours") < at(", settings") && at(", settings") < at("? help"));
    // Wider, the timer's too (it goes before settings, §4.1).
    let (m, _) = model(100, 30, 7);
    let buf = draw(&m, 100, 30);
    let line = row(&buf, m.layout.status.unwrap().y);
    let at = |h: &str| line.find(h).unwrap_or_else(|| panic!("{h:?} in {line:?}"));
    assert!(at("p colours") < at("Space timer") && at("Space timer") < at(", settings"));
}

fn key(k: &str) -> Event {
    let code = match k {
        "Space" => KeyCode::Char(' '),
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
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
        ("Space", Action::PomodoroToggle),
        (",", Action::Settings),
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
            "save" => Action::Keep,
            "cancel" => Action::Close,
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
            "playlists" => Action::Player(PlayerKey::Playlists),
            "back" => Action::Close,
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

/// The cell showing `glyph` in the music widget's rect, if drawn.
fn find_in_music(m: &Model, buf: &Buffer, glyph: &str) -> Option<(u16, u16)> {
    let music = crate::dock::by_name("music").unwrap().0;
    let rect = m
        .layout
        .panel
        .iter()
        .chain(&m.layout.on_lava)
        .flat_map(|s| &s.items)
        .find(|p| p.widget == music)?
        .rect;
    rect.positions()
        .find(|p| buf[(p.x, p.y)].symbol() == glyph)
        .map(|p| (p.x, p.y))
}

/// The mouse (lava-75z.5): a press lands on exactly what the music widget
/// drew there, beside the lamp and on the lava, at several sizes.
#[test]
fn presses_land_on_what_the_music_widget_draws() {
    for (cols, rows, lava) in [
        (80, 24, false),
        (120, 36, false),
        (120, 36, true),
        (60, 24, true),
    ] {
        let ctx = format!("{cols}x{rows} lava {lava}");
        let (mut m, t0) = model(cols, rows, 7);
        spotify(&mut m, t0, true);
        if lava {
            m.update(Action::Place("music"), t0);
            m.update(Action::Resize, t0);
        }
        let buf = draw(&m, cols, rows);

        let (x, y) = find_in_music(&m, &buf, "≡").unwrap_or_else(|| panic!("{ctx}: no ≡"));
        m.update(Action::Press { col: x, row: y }, t0);
        assert!(matches!(m.overlay, Overlay::Library(_)), "{ctx}");
        m.update(Action::Close, t0);

        let (x, y) = find_in_music(&m, &buf, "♥").unwrap_or_else(|| panic!("{ctx}: no ♥"));
        m.update(Action::Press { col: x, row: y }, t0);
        assert_eq!(m.liked(), Some(false), "{ctx}");

        let (x, y) = find_in_music(&m, &buf, "‖").unwrap_or_else(|| panic!("{ctx}: no ‖"));
        m.update(Action::Press { col: x, row: y }, t0);
        let snap = m.music.snapshot.as_ref().unwrap();
        assert_eq!(snap.status, Status::Paused, "{ctx}");

        // The last cell of the bar: seek to (almost) the end.
        let buf = draw(&m, cols, rows);
        let music = crate::dock::by_name("music").unwrap().0;
        let rect = m
            .layout
            .panel
            .iter()
            .chain(&m.layout.on_lava)
            .flat_map(|s| &s.items)
            .find(|p| p.widget == music)
            .unwrap()
            .rect;
        let end = rect
            .positions()
            .filter(|p| buf[(p.x, p.y)].symbol() == "─")
            .max_by_key(|p| p.x)
            .unwrap_or_else(|| panic!("{ctx}: no bar"));
        m.update(
            Action::Press {
                col: end.x,
                row: end.y,
            },
            t0,
        );
        let pos = m.music.snapshot.as_ref().unwrap().position.as_secs();
        assert!(pos >= 380, "{ctx}: seeked to {pos}s of 402");
    }
}

/// Without mouse capture the widget shows no buttons, only a liked heart.
#[test]
fn no_mouse_no_buttons() {
    let (mut m, t0) = model(80, 24, 7);
    m.settings.input.mouse = false;
    spotify(&mut m, t0, true);
    let buf = draw(&m, 80, 24);
    for glyph in ["◂◂", "◂", "≡"] {
        assert!(find_in_music(&m, &buf, glyph).is_none(), "{glyph}");
    }
    let (heart_x, y) = find_in_music(&m, &buf, "♥").expect("a liked heart");
    // lava-1xk.32: the row says how to reach the player keys instead of
    // sitting empty (it read as buttons the terminal failed to draw), and
    // never runs into the heart.
    let cells: Vec<&str> = (0..80).map(|x| buf[(x, y)].symbol()).collect();
    let line = cells.concat();
    assert!(line.contains("Shift+A music keys"), "{line:?}");
    assert_eq!(line.matches('+').count(), 1, "no add button: {line:?}");
    let last = (0..heart_x).rev().find(|&x| cells[usize::from(x)] != " ");
    assert!(last.is_some_and(|x| x + 3 <= heart_x), "{line:?}");
    // With the player keys on (`A`) the toast row's guide says the rest.
    m.update(Action::PlayerKeys, t0);
    let buf = draw(&m, 80, 24);
    let line: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
    assert!(!line.contains("Shift+A"), "{line:?}");
}

/// The status line's play glyph reads as a button, so a press on it plays
/// or pauses (lava-1xk.32: that's where people clicked).
#[test]
fn the_status_glyph_is_play_pause() {
    let (mut m, t0) = model(80, 24, 7);
    spotify(&mut m, t0, true);
    let buf = draw(&m, 80, 24);
    let (x, y) = find_in_music(&m, &buf, "▶").expect("the status glyph");
    m.update(Action::Press { col: x, row: y }, t0);
    let snap = m.music.snapshot.as_ref().unwrap();
    assert_eq!(snap.status, Status::Paused);
}

/// The music widget's form and rect, and where its inline cover is.
fn music_placed(m: &Model) -> (crate::dock::WidgetForm, Rect, Option<Rect>) {
    let music = crate::dock::by_name("music").unwrap().0;
    let p = m
        .layout
        .panel
        .iter()
        .chain(&m.layout.on_lava)
        .flat_map(|s| &s.items)
        .find(|p| p.widget == music)
        .expect("music placed");
    let cover = crate::dock::music::cover_rect(p.form, p.rect, ratatui::layout::Alignment::Left);
    (p.form, p.rect, cover)
}

/// Under a host embedding Ghostty's terminal (Ghostex), which drew none
/// of `◂◂ ‖ ▸▸ ≡` (lava-1xk.21), the controls use the safe set, and
/// presses still land on what's drawn.
#[test]
fn hosted_controls_use_safe_glyphs() {
    use crate::glyphs::SAFE;
    for lava in [false, true] {
        let (mut m, t0) = model(80, 24, 7);
        m.safe_glyphs = true;
        spotify(&mut m, t0, true);
        if lava {
            m.update(Action::Place("music"), t0);
            m.update(Action::Resize, t0);
        }
        let buf = draw(&m, 80, 24);
        // What the widget draws (on the lava, the lamp shows around it).
        let (form, rect, cover) = music_placed(&m);
        let mut drawn = Buffer::empty(buf.area);
        let look = Look {
            backdrop: if lava {
                Backdrop::Lava
            } else {
                Backdrop::Panel
            },
            align: ratatui::layout::Alignment::Left,
        };
        let music = crate::dock::by_name("music").unwrap().0;
        WIDGETS[music].draw(&m, form, rect, look, &mut drawn);
        let mut row = String::new();
        for p in rect.positions() {
            if cover.is_some_and(|c| c.contains(p)) {
                continue;
            }
            let sym = drawn[p].symbol();
            if p.y == rect.y + 3 {
                row.push_str(sym);
            }
            let ok = sym
                .chars()
                .all(|c| (c as u32) < 0x100 || "▶…━─".contains(c));
            assert!(ok, "lava {lava}: {sym:?} at {p:?}");
        }
        assert!(
            row.contains("«") && row.contains("||") && row.contains("»"),
            "{row}"
        );
        assert!(row.contains("<3") && row.contains(SAFE.playlists), "{row}");

        let (x, y) = find_in_music(&m, &buf, SAFE.playlists).expect("playlists");
        m.update(Action::Press { col: x, row: y }, t0);
        assert!(matches!(m.overlay, Overlay::Library(_)), "lava {lava}");
        m.update(Action::Close, t0);
        let (x, y) = find_in_music(&m, &buf, "|").expect("pause");
        m.update(Action::Press { col: x, row: y }, t0);
        let snap = m.music.snapshot.as_ref().unwrap();
        assert_eq!(snap.status, Status::Paused, "lava {lava}");
    }
}

/// Floating text leaves no block glyph of the lamp in or right around
/// it (lava-1xk.21): solid cells become plain backgrounds, and a half
/// block between two letters takes their backing.
#[test]
fn floating_text_has_no_block_glyphs_around_it() {
    let mut gaps = 0;
    for seed in [3, 7, 11, 19, 23, 42] {
        let (mut m, t0) = model(100, 30, seed);
        spotify(&mut m, t0, true);
        m.update(Action::Place("music"), t0);
        m.update(Action::Resize, t0);
        let buf = draw(&m, 100, 30);
        let (_, r, cover) = music_placed(&m);
        let around = Rect::new(r.x - 1, r.y.saturating_sub(1), r.width + 2, r.height + 2);
        for p in around.intersection(buf.area).positions() {
            assert_ne!(buf[p].symbol(), "█", "seed {seed} at {p:?}");
        }
        let text = |x: u16, y: u16| {
            let s = buf[(x, y)].symbol();
            s != " " && !"▀▄█".contains(s) && !cover.is_some_and(|c| c.contains((x, y).into()))
        };
        for y in r.top()..r.bottom() {
            for x in r.left() + 1..r.right() - 1 {
                if text(x - 1, y) && text(x + 1, y) && buf[(x, y)].symbol() != " " && !text(x, y) {
                    panic!("seed {seed}: block glyph between letters at ({x}, {y})");
                }
                if text(x - 1, y) && text(x + 1, y) && !text(x, y) {
                    gaps += 1;
                }
            }
        }
    }
    assert!(gaps > 0, "no gaps between letters were checked");
}

/// In macOS Terminal (block glyphs short of the cell's top; picked by
/// `display.cells = "auto"`) no cell of the frame, lamp, widgets or cover,
/// shows ink along its top edge; set to opaque, the lamp draws its usual
/// `█`s (lava-1xk.33).
#[test]
fn short_block_terminals_get_no_ink_along_cell_tops() {
    use crate::cells::Cells;
    use ratatui::style::Color;

    let inked_top = |buf: &Buffer| {
        buf.content.iter().any(|c| {
            c.fg != Color::Reset
                && c.bg != Color::Reset
                && ["█", "▀", "▛", "▜"].contains(&c.symbol())
        })
    };
    for seed in [3, 7, 11] {
        let (mut m, t0) = model(100, 30, seed);
        spotify(&mut m, t0, true);
        m.update(Action::Place("music"), t0);
        m.update(Action::Resize, t0);
        assert!(
            inked_top(&draw(&m, 100, 30)),
            "seed {seed}: no wax to check"
        );
        m.detected_cells = Cells::Background;
        assert_eq!(m.cells(), Cells::Background);
        assert!(!inked_top(&draw(&m, 100, 30)), "seed {seed}");
    }
}

/// Under a host embedding Ghostty's terminal (Ghostex), which draws none
/// of these, no frame shows them: music, lyrics, cover, pomodoro, chips,
/// toasts, lists and the status bar all take the safe set (lava-1xk.29).
/// With the rich set the same scenes do show them (the test sees them).
#[test]
fn hosted_frames_never_draw_symbols_the_host_lacks() {
    const LACKING: &str = "♪♡♥⇄↻◂▸‖≡";
    for safe in [true, false] {
        let mut seen = String::new();
        let mut check = |m: &Model, what: &str| {
            let area = m.layout.area;
            let buf = draw(m, area.width, area.height);
            for p in buf.area.positions() {
                for c in buf[p].symbol().chars().filter(|&c| LACKING.contains(c)) {
                    assert!(
                        !safe,
                        "{what} at {}x{}: {c:?} at {p:?}",
                        area.width, area.height
                    );
                    seen.push(c);
                }
            }
        };
        for (cols, rows) in [(120, 40), (60, 20), (34, 12)] {
            let (mut m, t0) = model(cols, rows, 3);
            m.safe_glyphs = safe;
            // Lyrics: a line, then a gap (the chip's note).
            lyrics(&mut m, t0, 1, 12);
            check(&m, "lyrics");
            lyrics(&mut m, t0, 0, 33);
            check(&m, "lyrics gap");

            // Music, its cover and the pomodoro; a like's toast.
            let (mut m, t0) = model(cols, rows, 3);
            m.safe_glyphs = safe;
            spotify(&mut m, t0, true);
            m.update(Action::Place("cover"), t0);
            m.update(Action::PomodoroToggle, t0);
            m.update(Action::Resize, t0);
            check(&m, "music, cover, pomodoro");
            m.update(Action::PlayerKeys, t0);
            m.update(Action::Player(crate::ui::keymap::PlayerKey::Shuffle), t0);
            m.update(Action::Player(crate::ui::keymap::PlayerKey::Repeat), t0);
            check(&m, "shuffle, repeat");
            m.update(Action::Player(crate::ui::keymap::PlayerKey::Like), t0);
            check(&m, "like toast");
            // Lists: the library, the settings screen, a picker.
            m.update(Action::Player(crate::ui::keymap::PlayerKey::Playlists), t0);
            m.update(Action::Resize, t0);
            check(&m, "library");
            m.update(Action::Close, t0);
            m.update(Action::Back, t0);
            m.update(Action::Settings, t0);
            check(&m, "settings");
            m.update(Action::Close, t0);
            m.update(Action::StylePicker, t0);
            check(&m, "picker");
            m.update(Action::Close, t0);
            // Paused, frozen.
            m.update(Action::PomodoroToggle, t0);
            m.update(Action::Freeze, t0);
            m.toast = None;
            check(&m, "paused, frozen");

            // No player: the message and the chip.
            let (mut m, t0) = model(cols, rows, 3);
            m.safe_glyphs = safe;
            let gone = Status::Unavailable(crate::media::Unavailable::NotRunning);
            music_track(&mut m, t0, gone, 1, "fake:1");
            m.update(Action::Place("lyrics"), t0);
            m.update(Action::Place("cover"), t0);
            m.update(Action::Resize, t0);
            check(&m, "no player");
        }
        if !safe {
            for c in "♪♡⇄↻◂▸‖≡".chars() {
                assert!(seen.contains(c), "the rich set's {c:?} was never drawn");
            }
        }
    }
}
