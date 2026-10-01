//! The app loop: input → fixed-step simulation → draw, paced to a target fps.
//!
//! Real time is fed into a [`FixedStep`] accumulator so the simulation always
//! advances in `SIM_HZ` steps, independent of how fast frames are drawn.
//! Between frames the loop sleeps inside `event::poll`, so input is handled
//! the moment it arrives without busy-waiting.

use std::io;
use std::time::Instant;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event;

use crate::config::Config;
use crate::sim::World;
use crate::timing::{FixedStep, FpsMeter, FramePacer};
use crate::ui::{self, Action, Scene};

/// Simulation rate. Fixed; unrelated to the render frame rate.
const SIM_HZ: u32 = 120;

pub fn run(terminal: &mut DefaultTerminal, config: &Config) -> io::Result<()> {
    let mut world = World::default();
    let mut sim_clock = FixedStep::new(SIM_HZ);
    let mut pacer = FramePacer::new(config.fps, Instant::now());
    let mut fps = FpsMeter::default();
    let mut last = Instant::now();
    let mut frames = 0;

    loop {
        match wait_for_input(pacer.deadline())? {
            Some(Action::Quit) => return Ok(()),
            Some(Action::Redraw) | None => {}
        }

        let now = Instant::now();
        for _ in 0..sim_clock.advance(now - last) {
            world.step(sim_clock.dt_secs());
        }
        last = now;

        let scene = Scene {
            time: world.time + sim_clock.alpha() * sim_clock.dt_secs(),
            fps: fps.fps(),
            minimal: config.minimal,
        };
        terminal.draw(|frame| ui::draw(frame, &scene))?;

        let drawn = Instant::now();
        fps.tick(drawn);
        pacer.frame_done(drawn);
        frames += 1;
        if config.max_frames.is_some_and(|max| frames >= max) {
            return Ok(());
        }
    }
}

/// Handle input until `deadline` (the next frame). Returns early if an event
/// needs acting on now: a quit, or a resize we want to redraw immediately.
fn wait_for_input(deadline: Instant) -> io::Result<Option<Action>> {
    while event::poll(deadline.saturating_duration_since(Instant::now()))? {
        if let Some(action) = ui::action_for(&event::read()?) {
            return Ok(Some(action));
        }
    }
    Ok(None)
}
