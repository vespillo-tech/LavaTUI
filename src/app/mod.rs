//! The app loop: wait for input until the next frame deadline → update the
//! [`Model`] → tick it → draw. All state and logic live in `model`; this
//! file only owns the terminal, the wall clock and the frame pacing.
//!
//! Input is handled the moment it arrives (the loop sleeps inside
//! `event::poll`), and any input draws a frame immediately, so a key shows
//! up within one frame (§7). Bursts (resize storms, held keys) are drained
//! before drawing, so only the final state is drawn.

mod model;
#[cfg(test)]
mod tests;

use std::io::{self, Write};
use std::time::{Duration, Instant, SystemTime};

use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::{execute, terminal};
use ratatui::layout::Rect;
use ratatui::{DefaultTerminal, Terminal};

pub use model::{LocalTime, Model, Overlay, Picker, TOAST_TIME, Toast};

use crate::clock::ClockTime;
use crate::config::Session;
use crate::config::store::Store;
use crate::render::LampState;
use crate::timing::{FpsMeter, FramePacer};
use crate::ui::{self, keymap};

/// Run the app. `panic_after` (hidden `--panic-after`) panics after that
/// many frames, to check the terminal is restored on a crash.
pub fn run(
    terminal: &mut DefaultTerminal,
    session: &Session,
    panic_after: Option<u64>,
) -> io::Result<()> {
    let store = Store::new(session.config_path.clone());
    let size = terminal.size()?;
    let mut model = Model::new(
        session,
        store,
        Rect::new(0, 0, size.width, size.height),
        reported_cell_aspect(),
        local_time(),
        session.seed.unwrap_or_else(time_seed),
        Instant::now(),
    );
    let modes = TerminalModes::enable(model.settings.input.mouse)?;
    let result = run_loop(terminal, &mut model, session.max_frames, panic_after);
    model.save();
    drop(modes);
    result
}

/// Focus reports (always: they let us drop to 10 fps in the background;
/// terminals that lack them just never send any) and mouse capture (if
/// enabled), for as long as this lives. Dropping it turns them off on every
/// way out: return, error or unwind. `main`'s panic hook covers panics
/// before the screen is restored.
struct TerminalModes;

impl TerminalModes {
    fn enable(mouse: bool) -> io::Result<Self> {
        // Guard first, so a failure half-way still switches off what's on.
        let guard = TerminalModes;
        execute!(io::stdout(), event::EnableFocusChange)?;
        if mouse {
            execute!(io::stdout(), event::EnableMouseCapture)?;
        }
        Ok(guard)
    }
}

impl Drop for TerminalModes {
    fn drop(&mut self) {
        disable_terminal_modes();
    }
}

/// Switch off focus reports and mouse capture. Harmless if they're off:
/// it's only a few escape codes, so it doesn't need to know what was on.
pub fn disable_terminal_modes() {
    let _ = execute!(
        io::stdout(),
        event::DisableMouseCapture,
        event::DisableFocusChange
    );
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    model: &mut Model,
    max_frames: Option<u64>,
    panic_after: Option<u64>,
) -> io::Result<()> {
    let mut fps = model.target_fps();
    let mut pacer = FramePacer::new(fps, Instant::now());
    let mut meter = FpsMeter::default();
    let mut frames = 0;
    let mut lamp = LampState::default();
    let mut events = TerminalEvents;

    loop {
        if wait_for_input(&mut events, model, pacer.deadline())? {
            // Input: draw now, not at the next deadline.
            pacer = FramePacer::new(fps, Instant::now());
        }
        if model.quit {
            return Ok(());
        }
        if std::mem::take(&mut model.clear) {
            full_repaint(terminal)?;
        }

        let started = Instant::now();
        draw_frame(terminal, model, &mut lamp, started, local_time())?;
        if std::mem::take(&mut model.bell) {
            io::stdout().write_all(b"\x07")?;
        }

        let drawn = Instant::now();
        meter.tick(drawn);
        model.stats.fps = meter.fps();
        model.stats.frame_ms = (drawn - started).as_secs_f64() * 1e3;
        if model.target_fps() != fps {
            fps = model.target_fps();
            pacer = FramePacer::new(fps, drawn);
        } else {
            pacer.frame_done(drawn);
        }
        frames += 1;
        if panic_after.is_some_and(|n| frames >= n) {
            panic!("--panic-after {frames}: deliberate panic");
        }
        if max_frames.is_some_and(|max| frames >= max) {
            return Ok(());
        }
    }
}

/// Tick the model and draw it, both at the size this frame is drawn at.
///
/// `Terminal::draw` first matches its buffer to the terminal's current
/// size, so `frame.area()` is the only size that is safe to lay out for:
/// a size read any earlier can already be stale (a window-edge drag), and
/// its rects would land outside the buffer.
fn draw_frame<B: Backend>(
    terminal: &mut Terminal<B>,
    model: &mut Model,
    lamp: &mut LampState,
    now: Instant,
    local: LocalTime,
) -> Result<(), B::Error> {
    terminal.draw(|frame| {
        model.tick(now, frame.area(), local);
        ui::draw(frame, model, lamp);
    })?;
    Ok(())
}

/// Clear the screen and repaint every cell on the next draw (ctrl-l).
///
/// Not `Terminal::clear`: that asks the terminal where its cursor is, a
/// blocking round trip (slow over SSH) that errors out if the terminal or a
/// multiplexer never answers. `resize` clears and resets the back buffer
/// without asking anything. Resizes need none of this: `Terminal::draw`
/// does the same whenever the size changes.
fn full_repaint<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    let area: Rect = terminal.size()?.into();
    terminal.resize(area)
}

/// Where input comes from: the terminal, or a script in tests.
trait Events {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
}

struct TerminalEvents;

impl Events for TerminalEvents {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

/// Handle input until `deadline`. Returns `true` as soon as anything was
/// handled (after draining whatever else is already queued).
///
/// The wait is recomputed from `deadline` every time round, so a stream of
/// events we ignore (mouse motion) can't hold the frame back, and draining
/// stops at the deadline, so a flood of keys can't either.
fn wait_for_input(
    events: &mut impl Events,
    model: &mut Model,
    deadline: Instant,
) -> io::Result<bool> {
    let mut handled = false;
    loop {
        let timeout = if handled {
            Duration::ZERO
        } else {
            deadline.saturating_duration_since(Instant::now())
        };
        if !events.poll(timeout)? {
            return Ok(handled);
        }
        let event = events.read()?;
        if let Event::Resize(..) = event {
            model.cell_aspect =
                reported_cell_aspect().unwrap_or(model.settings.display.cell_aspect);
        }
        if let Some(action) = keymap::action_for(&event, model.input_mode()) {
            model.update(action, Instant::now());
            handled = true;
        }
        if model.quit || Instant::now() >= deadline {
            return Ok(handled);
        }
    }
}

/// Cell height ÷ width from the terminal's pixel size, when it reports one
/// (§2.3), clamped to a sane range.
fn reported_cell_aspect() -> Option<f64> {
    let size = terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    let cell_w = f64::from(size.width) / f64::from(size.columns);
    let cell_h = f64::from(size.height) / f64::from(size.rows);
    Some((cell_h / cell_w).clamp(1.6, 2.6))
}

/// Local wall-clock time and date for the clock face.
fn local_time() -> LocalTime {
    let now = jiff::Zoned::now();
    let time = ClockTime::new(now.hour() as u8, now.minute() as u8, now.second() as u8)
        .unwrap_or(ClockTime::from_secs_of_day(0));
    LocalTime {
        time,
        date: now.strftime("%a %-d %b").to_string().to_lowercase(),
        wall: SystemTime::now(),
    }
}

/// A different lamp every launch, unless `--seed` pins it.
fn time_seed() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}
