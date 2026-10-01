//! The app loop: wait for input until the next frame deadline → update the
//! [`Model`] → tick it → draw. All state and logic live in `model`; this
//! file only owns the terminal, the wall clock and the frame pacing.
//!
//! Input is handled the moment it arrives (the loop sleeps inside
//! `event::poll`), and any input draws a frame immediately, so a key shows
//! up within one frame (§7). Bursts (resize storms, held keys) are drained
//! before drawing, so only the final state is drawn.

mod model;

use std::io::{self, Write};
use std::time::{Duration, Instant, SystemTime};

use ratatui::DefaultTerminal;
use ratatui::crossterm::{event, execute, terminal};
use ratatui::layout::Rect;

pub use model::{FLASH_TIME, LocalTime, Model, Overlay, Picker, TOAST_TIME, Toast};

use crate::clock::ClockTime;
use crate::config::Session;
use crate::config::store::Store;
use crate::render::LampState;
use crate::timing::{FpsMeter, FramePacer};
use crate::ui::{self, keymap};

pub fn run(terminal: &mut DefaultTerminal, session: &Session) -> io::Result<()> {
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
    let mouse = model.settings.input.mouse;
    // Focus reports let us drop to 10 fps in the background; terminals
    // that don't support them just never send any.
    execute!(io::stdout(), event::EnableFocusChange)?;
    if mouse {
        execute!(io::stdout(), event::EnableMouseCapture)?;
    }
    let result = run_loop(terminal, &mut model, session.max_frames);
    model.save();
    if mouse {
        execute!(io::stdout(), event::DisableMouseCapture)?;
    }
    execute!(io::stdout(), event::DisableFocusChange)?;
    result
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    model: &mut Model,
    max_frames: Option<u64>,
) -> io::Result<()> {
    let mut fps = model.target_fps();
    let mut pacer = FramePacer::new(fps, Instant::now());
    let mut meter = FpsMeter::default();
    let mut frames = 0;
    let mut lamp = LampState::default();

    loop {
        if wait_for_input(model, pacer.deadline())? {
            // Input: draw now, not at the next deadline.
            pacer = FramePacer::new(fps, Instant::now());
        }
        if model.quit {
            return Ok(());
        }
        if std::mem::take(&mut model.clear) {
            terminal.clear()?;
        }

        let size = terminal.size()?;
        let started = Instant::now();
        model.tick(
            started,
            Rect::new(0, 0, size.width, size.height),
            local_time(),
        );
        if std::mem::take(&mut model.bell) {
            io::stdout().write_all(b"\x07")?;
        }
        terminal.draw(|frame| ui::draw(frame, model, &mut lamp))?;

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
        if max_frames.is_some_and(|max| frames >= max) {
            return Ok(());
        }
    }
}

/// Handle input until `deadline`. Returns `true` as soon as anything was
/// handled (after draining whatever else is already queued).
fn wait_for_input(model: &mut Model, deadline: Instant) -> io::Result<bool> {
    let mut timeout = deadline.saturating_duration_since(Instant::now());
    let mut handled = false;
    while event::poll(timeout)? {
        let event = event::read()?;
        if let event::Event::Resize(..) = event {
            model.cell_aspect =
                reported_cell_aspect().unwrap_or(model.settings.display.cell_aspect);
        }
        if let Some(action) = keymap::action_for(&event, model.input_mode()) {
            model.update(action, Instant::now());
            handled = true;
            timeout = Duration::ZERO;
        }
        if model.quit {
            break;
        }
    }
    Ok(handled)
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
    }
}

/// A different lamp every launch, unless `--seed` pins it.
fn time_seed() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}
