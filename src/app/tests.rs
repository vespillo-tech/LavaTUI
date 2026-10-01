//! The loop's terminal-facing paths against ratatui's `TestBackend`:
//! resizes between (and during) frames, overlays at degenerate sizes,
//! ctrl-l, and input that must not starve frames.

use std::cell::Cell;
use std::path::PathBuf;

use ratatui::backend::{ClearType, TestBackend, WindowSize};
use ratatui::buffer::Cell as BufCell;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Size};

use super::*;
use crate::ui::keymap::Action;

fn temp_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lavatui-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("config.toml")
}

fn local() -> LocalTime {
    LocalTime {
        time: ClockTime::new(14, 32, 7).unwrap(),
        date: "thu 1 oct".into(),
        wall: SystemTime::UNIX_EPOCH,
    }
}

fn model(name: &str, cols: u16, rows: u16) -> (Model, Instant) {
    let t0 = Instant::now();
    let model = Model::new(
        &Session::default(),
        Store::new(Some(temp_config(name))),
        Rect::new(0, 0, cols, rows),
        None,
        local(),
        7,
        t0,
    );
    (model, t0)
}

/// Small deterministic generator (xorshift), so storms are reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn range(&mut self, lo: u16, hi: u16) -> u16 {
        lo + (self.next() % u64::from(hi - lo + 1)) as u16
    }
}

/// Every overlay the app can show, plus none.
fn overlays() -> [Action; 5] {
    [
        Action::Close,
        Action::Help,
        Action::StylePicker,
        Action::FacePicker,
        Action::PalettePicker,
    ]
}

fn open(m: &mut Model, overlay: Action, now: Instant) {
    m.overlay = Overlay::None;
    if overlay != Action::Close {
        m.update(overlay, now);
    }
}

/// lava-ebq.4: the model ticked (laid out) at A, the terminal drawn at B.
/// Before the fix, a smaller B panicked with "index outside of buffer".
#[test]
fn drawing_at_a_size_other_than_the_ticked_one_never_panics() {
    let pairs = [
        ((80, 24), (80, 23)),
        ((100, 40), (80, 24)),
        ((80, 24), (10, 17)),
        ((220, 70), (5, 2)),
        ((120, 36), (120, 1)),
        ((50, 16), (49, 16)),
        ((30, 10), (200, 60)),
        ((80, 24), (1, 1)),
        ((300, 90), (40, 14)),
    ];
    let (mut m, t0) = model("tick-a-draw-b", 80, 24);
    let mut lamp = LampState::default();
    let mut now = t0;
    for overlay in overlays() {
        open(&mut m, overlay, now);
        for ((aw, ah), (bw, bh)) in pairs {
            now += Duration::from_millis(16);
            m.tick(now, Rect::new(0, 0, aw, ah), local());
            let mut terminal = Terminal::new(TestBackend::new(bw, bh)).unwrap();
            terminal
                .draw(|frame| ui::draw(frame, &m, &mut lamp))
                .unwrap();
        }
    }
}

/// A resize storm through the real frame path: the backend changes size
/// between frames *and* between the model's last layout and the draw,
/// with overlays and modes switching underneath.
#[test]
fn resize_storm_through_the_frame_path() {
    for seed in 1..=4u64 {
        let (mut m, t0) = model(&format!("storm-{seed}"), 80, 24);
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut lamp = LampState::default();
        let mut now = t0;
        let toggles = [
            Action::ToggleMinimal,
            Action::ToggleStatusBar,
            Action::Place("clock"),
            Action::Place("pomodoro"),
            Action::NextAnchor,
            Action::PomodoroToggle,
            Action::DebugHud,
            Action::NextFace,
            Action::NextStyle,
        ];
        for i in 0..250 {
            let (w, h) = (rng.range(1, 300), rng.range(1, 90));
            terminal.backend_mut().resize(w, h);
            m.update(Action::Resize, now);
            if i % 7 == 0 {
                let t = toggles[(rng.next() % toggles.len() as u64) as usize];
                m.update(t, now);
            }
            if i % 11 == 0 {
                let o = overlays()[(rng.next() % 5) as usize];
                open(&mut m, o, now);
            }
            // Sometimes the terminal moves again after input was handled,
            // right before the frame: the draw must follow it.
            if rng.next().is_multiple_of(3) {
                terminal
                    .backend_mut()
                    .resize(rng.range(1, 300), rng.range(1, 90));
            }
            now += Duration::from_millis(rng.next() % 20);
            draw_frame(&mut terminal, &mut m, &mut lamp, now, local()).unwrap();
            let size = terminal.size().unwrap();
            assert_eq!(
                m.layout.area,
                Rect::new(0, 0, size.width, size.height),
                "the model is laid out for the size that was drawn"
            );
        }
    }
}

/// lava-ebq.19 (4): zero-height (and zero-width) frames with every overlay
/// open draw nothing and don't panic.
#[test]
fn degenerate_sizes_with_every_overlay_open() {
    let (mut m, t0) = model("zero-height", 80, 24);
    let mut lamp = LampState::default();
    for overlay in overlays() {
        open(&mut m, overlay, t0);
        let sizes = (1..=300).map(|w| (w, 0)).chain((0..=90).map(|h| (0, h)));
        for (w, h) in sizes {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            draw_frame(&mut terminal, &mut m, &mut lamp, t0, local()).unwrap();
            // And drawn straight after a layout for a normal size.
            m.tick(t0, Rect::new(0, 0, 80, 24), local());
            terminal
                .draw(|frame| ui::draw(frame, &m, &mut lamp))
                .unwrap();
        }
    }
}

/// A backend that fails the test if anything asks where the cursor is:
/// that query is a blocking round trip to the terminal (lava-ebq.6).
struct NoCursorQueries {
    inner: TestBackend,
    clears: Cell<u32>,
}

impl Backend for NoCursorQueries {
    type Error = <TestBackend as Backend>::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a BufCell)>,
    {
        self.inner.draw(content)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        panic!("cursor position queried: a blocking round trip to the terminal");
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.clears.set(self.clears.get() + 1);
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.clears.set(self.clears.get() + 1);
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// ctrl-l and resizes repaint everything without a cursor-position query.
#[test]
fn redraw_and_resize_never_query_the_cursor() {
    let (mut m, t0) = model("no-dsr", 80, 24);
    let backend = NoCursorQueries {
        inner: TestBackend::new(80, 24),
        clears: Cell::new(0),
    };
    let mut terminal = Terminal::new(backend).unwrap();
    let mut lamp = LampState::default();
    draw_frame(&mut terminal, &mut m, &mut lamp, t0, local()).unwrap();

    // ctrl-l: a full clear, and every cell is sent again.
    m.update(Action::Redraw, t0);
    assert!(std::mem::take(&mut m.clear));
    let clears = terminal.backend().clears.get();
    full_repaint(&mut terminal).unwrap();
    assert_eq!(terminal.backend().clears.get(), clears + 1);
    let completed = terminal
        .draw(|frame| {
            m.tick(t0, frame.area(), local());
            ui::draw(frame, &m, &mut lamp);
        })
        .unwrap()
        .buffer
        .clone();
    assert_eq!(completed.area, Rect::new(0, 0, 80, 24));

    // A resize sets no clear flag of its own; the draw handles it.
    for (w, h) in [(60, 20), (120, 40), (3, 2)] {
        terminal.backend_mut().inner.resize(w, h);
        m.update(Action::Resize, t0);
        assert!(!m.clear, "a resize doesn't ask for an extra clear");
        draw_frame(&mut terminal, &mut m, &mut lamp, t0, local()).unwrap();
    }
}

/// Plays back events at a fixed rate on a fake clock: polling advances
/// it instead of sleeping, so timing checks are exact under any load
/// (lava-ebq.42).
struct Stream {
    now: Instant,
    every: Duration,
    next_at: Instant,
    event: Event,
}

impl Stream {
    fn new(every: Duration, event: Event) -> Self {
        let now = Instant::now();
        Stream {
            now,
            every,
            next_at: now,
            event,
        }
    }
}

impl Events for Stream {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        let wait = self.next_at.saturating_duration_since(self.now);
        if wait > timeout {
            self.now += timeout;
            return Ok(false);
        }
        self.now += wait;
        Ok(true)
    }

    fn read(&mut self) -> io::Result<Event> {
        self.next_at = self.now + self.every;
        Ok(self.event.clone())
    }

    fn now(&self) -> Instant {
        self.now
    }
}

/// lava-ebq.19 (1): a 100 Hz stream of events that map to nothing (mouse
/// motion under capture) must not hold the frame back past its deadline.
#[test]
fn ignored_events_dont_starve_frames() {
    let (mut m, _) = model("starve", 80, 24);
    let mut events = Stream::new(
        Duration::from_millis(10),
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 3,
            row: 3,
            modifiers: KeyModifiers::NONE,
        }),
    );
    let mut replies = ReplyFilter::default();
    for _ in 0..5 {
        let deadline = events.now + Duration::from_millis(50);
        assert!(!wait_for_input(&mut events, &mut replies, &mut m, deadline).unwrap());
        assert_eq!(events.now, deadline, "frame held back");
    }
}

/// A flood of real keys (a held key at a fast repeat rate) is handled, and
/// the frame still comes by its deadline.
#[test]
fn a_key_flood_still_draws_by_the_deadline() {
    let (mut m, _) = model("flood", 80, 24);
    let mut events = Stream::new(
        Duration::from_millis(2),
        Event::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
    );
    let deadline = events.now + Duration::from_millis(40);
    let mut replies = ReplyFilter::default();
    assert!(wait_for_input(&mut events, &mut replies, &mut m, deadline).unwrap());
    assert!(events.now <= deadline, "frame held back");
}

/// Events that all arrived together (one read), then nothing.
struct Burst {
    now: Instant,
    events: std::collections::VecDeque<Event>,
}

impl Events for Burst {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.events.is_empty() {
            self.now += timeout;
        }
        Ok(!self.events.is_empty())
    }

    fn read(&mut self) -> io::Result<Event> {
        Ok(self.events.pop_front().expect("polled first"))
    }

    fn now(&self) -> Instant {
        self.now
    }
}

/// What the replies could change: everything a key in the keymap touches
/// (heat and speed are in the settings).
fn state(m: &Model) -> String {
    format!(
        "{:?} {:?} {:?} {} {} {} {} {:?}",
        m.settings,
        m.overlay,
        m.pomodoro.status(),
        m.face.name(),
        m.frozen,
        m.hud,
        m.quit,
        m.toast.as_ref().map(|t| t.text.clone()),
    )
}

/// lava-ebq.31: DCS/OSC/APC/DA replies, as crossterm reads them, change
/// nothing and never quit; real keys in the same burst still act.
#[test]
fn terminal_replies_are_not_keys() {
    use super::replies::tests::{REPLIES, crossterm_events};
    for reply in REPLIES {
        let (mut m, t0) = model("replies", 80, 24);
        let before = state(&m);
        let mut events = Burst {
            now: t0,
            events: crossterm_events(reply).into(),
        };
        let deadline = t0 + Duration::from_millis(16);
        let handled =
            wait_for_input(&mut events, &mut ReplyFilter::default(), &mut m, deadline).unwrap();
        let shown = String::from_utf8_lossy(reply);
        assert!(!handled, "{shown:?} acted");
        assert_eq!(state(&m), before, "{shown:?} changed state");
    }
    // A reply between two real keys: both keys act, the reply doesn't.
    let (mut m, t0) = model("replies-mixed", 80, 24);
    let mut bytes = b"]".to_vec();
    bytes.extend_from_slice(b"\x1bP+q\x1b\\");
    bytes.extend_from_slice(b"z");
    let mut events = Burst {
        now: t0,
        events: crossterm_events(&bytes).into(),
    };
    let heat = m.settings.lamp.heat;
    wait_for_input(&mut events, &mut ReplyFilter::default(), &mut m, t0).unwrap();
    assert!(!m.quit);
    assert_eq!(m.settings.lamp.heat, heat + 1);
    assert!(m.frozen);
}

/// An actual input burst redraws at once without moving the scheduled grid.
#[test]
fn input_redraw_keeps_the_scheduled_next_frame() {
    let (mut model, t0) = model("input-grid", 80, 24);
    let mut pacer = FramePacer::new(60, t0);
    let deadline = pacer.deadline();
    let mut input = Burst {
        now: t0 + Duration::from_millis(10),
        events: [Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::NONE,
        ))]
        .into(),
    };
    assert!(
        wait_for_input(
            &mut input,
            &mut ReplyFilter::default(),
            &mut model,
            deadline
        )
        .unwrap()
    );
    pacer.frame_done(input.now + Duration::from_millis(1));
    assert_eq!(pacer.deadline(), deadline);
    assert!(pacer.deadline() - input.now < Duration::from_millis(7));
}

#[test]
fn cleanup_ends_sync_and_shows_cursor_without_using_the_frame_buffer() {
    if !output::ansi_output() {
        return;
    }
    let mut bytes = Vec::new();
    restore_modes(&mut bytes).unwrap();
    assert!(bytes.ends_with(b"\x1b[?2026l\x1b[?25h"));
}
