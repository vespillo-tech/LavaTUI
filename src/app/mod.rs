//! The app loop: wait for input until the next frame deadline → update the
//! [`Model`] → tick it → draw. All state and logic live in `model`; this
//! file only owns the terminal, the wall clock and the frame pacing.
//!
//! Input is handled the moment it arrives (the loop sleeps inside
//! `event::poll`), and input draws a frame immediately, so a key shows up
//! within one frame (§7), unless a frame started less than a period ago:
//! then it waits out the period, handling whatever else arrives, so input
//! never draws faster than the target fps. Bursts (resize storms, held
//! keys) are drained before drawing, so only the final state is drawn. Terminal replies that
//! crossterm reads as keys are filtered out first ([`replies`]).

mod model;
mod output;
mod replies;
#[cfg(test)]
mod tests;
mod trace;
mod wake;

use std::io::{self, Write};
use std::time::{Duration, Instant, SystemTime};

use output::AppTerminal;
pub use output::new_terminal;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::{execute, queue, terminal};
use ratatui::layout::Rect;
use std::path::Path;

pub use model::{
    Account, Adding, Fetch, Item, Kind, ListKind, ListView, LocalTime, Model, Overlay, Page,
    Picker, PickerKind, Row, SettingsView, Stage, TOAST_FADE, TOAST_TIME, Toast,
};

use crate::clock::ClockTime;
use crate::config::Session;
use crate::config::store::Store;
use crate::render::LampState;
use crate::timing::{FpsMeter, FramePacer, input_frame_at};
use crate::ui::{self, keymap};
use replies::ReplyFilter;

/// Run the app. `panic_after` (hidden `--panic-after`) panics after that
/// many frames, to check the terminal is restored on a crash. Returns the
/// config problems to print after the terminal is restored.
pub fn run(
    terminal: &mut AppTerminal,
    session: &Session,
    panic_after: Option<u64>,
    trace_path: Option<&Path>,
) -> io::Result<Vec<String>> {
    let store = Store::new(session.config_path.clone());
    let size = terminal.size()?;
    let cell = reported_cell();
    let mut model = Model::new(
        session,
        store,
        Rect::new(0, 0, size.width, size.height),
        cell.map(|c| c.aspect),
        local_time(),
        session.seed.unwrap_or_else(time_seed),
        Instant::now(),
    );
    model.option_drag = option_drag(std::env::var("TERM_PROGRAM").ok().as_deref());
    model.cell_px = cell.map(|c| c.px);
    model.background_saves()?;
    let modes = TerminalModes::enable(model.settings.input.mouse)?;
    if let Some(probe) = &model.probe {
        // Does the terminal really show pictures? Asked once; the answer
        // comes back as input, whenever it does.
        let mut out = io::stdout();
        out.write_all(&probe.query())?;
        out.flush()?;
    }
    let mut trace = trace::Trace::new(trace_path)?;
    let result = run_loop(
        terminal,
        &mut model,
        session.max_frames,
        panic_after,
        &mut trace,
    );
    let traced = trace.finish();
    model.finish_saves();
    drop(modes);
    result.and(traced).map(|()| model.config_report())
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
        // Pastes arrive whole (the settings screen's text field). Legacy
        // Windows consoles can't: there a paste is typed key by key.
        if output::ansi_output() {
            execute!(io::stdout(), event::EnableBracketedPaste)?;
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
    let _ = restore_modes(io::stdout());
}

fn restore_modes(mut writer: impl Write) -> io::Result<()> {
    if output::ansi_output() {
        // Our kitty images, if any were sent.
        writer.write_all(&crate::graphics::cleanup())?;
    }
    queue!(
        writer,
        event::DisableMouseCapture,
        event::DisableFocusChange
    )?;
    if output::ansi_output() {
        queue!(
            writer,
            event::DisableBracketedPaste,
            terminal::EndSynchronizedUpdate
        )?;
    }
    execute!(writer, ratatui::crossterm::cursor::Show)
}

fn run_loop(
    terminal: &mut AppTerminal,
    model: &mut Model,
    max_frames: Option<u64>,
    panic_after: Option<u64>,
    trace: &mut trace::Trace,
) -> io::Result<()> {
    let mut fps = model.target_fps();
    let mut pacer = FramePacer::new(fps, Instant::now());
    let mut meter = FpsMeter::default();
    let mut frames = 0;
    let mut lamp = LampState::default();
    let mut events = TerminalEvents { precise: true };
    let mut priority = crate::thread_qos::UiPriority::new();
    let mut replies = ReplyFilter::default();
    let mut last_drawn = None;
    let mut mouse = model.settings.input.mouse;
    let mut last_started = None;

    loop {
        let idle = model.idle_until();
        events.precise = model.focused && idle.is_none();
        priority.update(events.precise);
        let deadline = idle.unwrap_or_else(|| pacer.deadline());
        let wait_start = Instant::now();
        let wait_cpu_start = trace.enabled().then(crate::thread_qos::cpu_ns);
        let input = wait_for_input(&mut events, &mut replies, model, deadline, last_started)?;
        let wait_cpu_us = wait_cpu_start.map_or(0, |start| {
            crate::thread_qos::cpu_ns().saturating_sub(start) / 1000
        });
        let wait_end = Instant::now();
        // Input draws off the grid (at most a frame a period), while the
        // scheduled grid stays put.
        if model.quit {
            return Ok(());
        }
        let started = Instant::now();
        last_started = Some(started);
        terminal.backend_mut().writer_mut().begin_frame()?;
        if std::mem::take(&mut model.clear) {
            full_repaint(terminal)?;
            model.inline.invalidate();
        }
        let timings = draw_frame(terminal, model, &mut lamp, started, local_time())?;
        // Pictures after the cells, in the same synchronized update.
        model.kitty.write(terminal.backend_mut())?;
        model.inline.write(terminal.backend_mut())?;
        if std::mem::take(&mut model.bell) {
            terminal.backend_mut().write_all(b"\x07")?;
        }
        if let Some(text) = model.copy.take() {
            terminal.backend_mut().write_all(&osc52(&text))?;
        }
        // The settings screen turns the mouse on and off as you watch.
        if model.settings.input.mouse != mouse {
            mouse = model.settings.input.mouse;
            let writer = terminal.backend_mut();
            if mouse {
                queue!(writer, event::EnableMouseCapture)?;
            } else {
                queue!(writer, event::DisableMouseCapture)?;
            }
        }

        let output = terminal.backend_mut().writer_mut().finish_frame()?;
        let drawn = Instant::now();
        let dt = last_drawn.map_or(Duration::ZERO, |at| drawn - at);
        last_drawn = Some(drawn);
        trace.record(trace::Frame {
            frame: frames,
            wait_start,
            wait_end,
            wait_cpu_us,
            started,
            drawn,
            deadline,
            interval: dt,
            input,
            tick_us: timings.0,
            draw_us: timings.1,
            total_us: (drawn - started).as_micros() as u64,
            sim_steps: model.stats.sim_steps,
            sim_dt: model.stats.sim_dt,
            sim_feed_s: model.stats.sim_feed_s,
            save_us: model.stats.save_us,
            output,
            fps,
        });
        meter.tick(drawn);
        model.stats.fps = meter.fps();
        model.frame_drawn((drawn - started).as_secs_f64() * 1e3, dt, drawn);
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
) -> Result<(u64, u64), B::Error> {
    let mut timings = (0, 0);
    terminal.draw(|frame| {
        let tick = Instant::now();
        model.tick(now, frame.area(), local);
        let draw = Instant::now();
        ui::draw(frame, model, lamp);
        // An iTerm2 / sixel picture's cells: skipped while it's up,
        // rewritten where it was.
        model.inline.settle(frame.buffer_mut());
        timings = (
            (draw - tick).as_micros() as u64,
            draw.elapsed().as_micros() as u64,
        );
    })?;
    Ok(timings)
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

/// Where input comes from, and the clock it's timed by: the terminal and
/// the real clock, or a script on a fake clock in tests.
trait Events {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
    fn now(&self) -> Instant;
}

struct TerminalEvents {
    precise: bool,
}

impl Events for TerminalEvents {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.precise {
            wake::poll(timeout)
        } else {
            event::poll(timeout)
        }
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Most events read as one burst. Far more than any terminal reply; it
/// only bounds the work between deadline checks.
const MAX_BURST: usize = 128;

/// Handle input until `deadline`. Returns `true` once anything was handled
/// and its frame may draw ([`input_frame_at`]: at once, or a period after
/// `last_frame`), having handled everything that arrived until then.
///
/// Events are read in bursts (everything already queued) and terminal
/// replies are dropped from each burst before any of it is dispatched.
/// The wait is recomputed from `deadline` every time round, so a stream of
/// events we ignore (mouse motion) can't hold the frame back, and draining
/// stops at the deadline, so a flood of keys can't either.
fn wait_for_input(
    events: &mut impl Events,
    replies: &mut ReplyFilter,
    model: &mut Model,
    deadline: Instant,
    last_frame: Option<Instant>,
) -> io::Result<bool> {
    let mut handled = false;
    // When to draw: the deadline, or sooner once input was handled.
    let mut draw_at = deadline;
    let mut burst = Vec::new();
    loop {
        let timeout = draw_at.saturating_duration_since(events.now());
        if !events.poll(timeout)? {
            return Ok(handled);
        }
        burst.push(events.read()?);
        while burst.len() < MAX_BURST && events.poll(Duration::ZERO)? {
            burst.push(events.read()?);
        }
        let now = events.now();
        replies.filter(&mut burst, now);
        let answers = replies.take();
        if !answers.is_empty() && model.terminal_replies(&answers) {
            handled = true;
        }
        for event in burst.drain(..) {
            if let Event::Resize(..) = event {
                let cell = reported_cell();
                model.cell_aspect = cell.map_or(model.settings.display.cell_aspect, |c| c.aspect);
                model.cell_px = cell.map(|c| c.px);
            }
            if let Event::Paste(text) = &event {
                model.paste(text, now);
                handled = true;
            }
            if let Some(action) = keymap::action_for(&event, model.input_mode()) {
                model.update(action, now);
                handled = true;
            }
            if model.quit {
                return Ok(handled);
            }
        }
        if handled {
            // Focus can change the fps: recompute with each burst.
            draw_at = input_frame_at(last_frame, model.target_fps(), now, deadline);
        }
        if now >= draw_at {
            return Ok(handled);
        }
    }
}

/// OSC 52: put `text` on the system clipboard, in terminals that allow it
/// (most do; tmux needs `set-clipboard on`).
fn osc52(text: &str) -> Vec<u8> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    format!("\x1b]52;c;{encoded}\x07").into_bytes()
}

/// Whether the terminal (`TERM_PROGRAM`) selects text past mouse capture
/// with option held rather than shift: macOS Terminal and iTerm2.
fn option_drag(term_program: Option<&str>) -> bool {
    matches!(term_program, Some("Apple_Terminal" | "iTerm.app"))
}

/// A cell's shape, from the terminal's pixel size.
#[derive(Debug, Clone, Copy)]
struct CellSize {
    /// Height ÷ width (§2.3), clamped to a sane range.
    aspect: f64,
    /// Whole pixels, width and height (rounded down: a sixel picture
    /// sized by them never spills past its cells).
    px: (u16, u16),
}

/// The cell's shape, when the terminal reports its size in pixels.
fn reported_cell() -> Option<CellSize> {
    let size = terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    let cell_w = f64::from(size.width) / f64::from(size.columns);
    let cell_h = f64::from(size.height) / f64::from(size.rows);
    Some(CellSize {
        aspect: (cell_h / cell_w).clamp(1.6, 2.6),
        px: (size.width / size.columns, size.height / size.rows),
    })
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
