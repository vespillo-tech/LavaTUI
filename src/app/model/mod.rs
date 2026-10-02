//! The app's state and everything that changes it.
//!
//! [`Model::update`] applies one [`Action`] (a key, the mouse, a resize);
//! [`Model::tick`] advances time once per frame (pomodoro, toasts, sim
//! steps, debounced config save) and recomputes the layout. Neither touches
//! the terminal: time and the local clock are passed in, and the side
//! effects the loop must perform (bell, full clear) are flags it drains.

mod actions;
mod library;
mod lyrics;
mod music;
mod pickers;
mod saving;

use std::time::{Duration, Instant, SystemTime};

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::clock::{
    self, ClockTime, Face, PhaseEnd, Pomodoro, PomodoroConfig, Status, format_remaining,
};
use crate::config::store::Store;
use crate::config::{self, CellsChoice, ColorChoice, Overridden, Session, Settings};
use crate::dock::cover::Caps;
use crate::dock::{Place, WIDGETS};
use crate::graphics::Kitty;
use crate::graphics::inline::Inline;
use crate::graphics::probe::Probe;
use crate::render::StyleId;
use crate::sim::{Field, HEAT_LEVELS, SimSpeed, World};
use crate::theme::{ColorDepth, Palette, Theme};
use crate::timing::{FixedStep, Quality};
use crate::ui::keymap::InputMode;
use crate::ui::layout::{self, DockItem, Layout, LayoutInput, SizeTier};

pub use library::{Account, Library, ListKind, ListView};
pub use lyrics::{Fetch, LyricsState};
pub use music::Music;
pub use pickers::{Picker, PickerKind};

/// Simulation rate. Fixed; unrelated to the render frame rate.
pub const SIM_HZ: u32 = 120;
/// Headless steps run at launch so the first frame already looks alive.
const PREWARM_STEPS: u32 = 600;
/// How long a toast stays up (§4.2).
pub const TOAST_TIME: Duration = Duration::from_millis(1400);
/// The end of a toast's time that it fades out over (truecolor).
pub const TOAST_FADE: Duration = Duration::from_millis(400);
/// Phase-change flash length (§4.5).
pub const FLASH_TIME: Duration = Duration::from_millis(600);
/// `r` must be pressed twice within this to reset the pomodoro.
const RESET_WINDOW: Duration = Duration::from_secs(2);
/// `r` presses closer together than this are a held key's auto-repeat
/// (typically 30–90 ms apart), not a deliberate double press.
const REPEAT_GAP: Duration = Duration::from_millis(150);
/// Wall time running ahead of `Instant` by at least this between two
/// frames means the machine was asleep (see [`Model::tick`]).
const SLEEP_MIN: Duration = Duration::from_secs(2);
/// Settings are written this long after the last change.
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// Speed changes ease in with this time constant (~95 % in 0.5 s).
const SPEED_EASE: f64 = 0.17;
/// Frame rate in the background (§7).
const UNFOCUSED_FPS: u32 = 10;
/// Frozen frames wake this long after the clock ticks over, so the new
/// minute (or second) is surely there to read.
const WAKE_SLACK: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help {
        scroll: u16,
    },
    Picker(Picker),
    /// The playlist browser / add-to-playlist picker.
    Library(ListView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub at: Instant,
}

/// Local wall-clock time, read by the loop and passed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTime {
    pub time: ClockTime,
    /// `thu 1 oct`.
    pub date: String,
    /// The system clock when this was read. Unlike `Instant` it keeps
    /// counting while the machine sleeps.
    pub wall: SystemTime,
}

/// Measured per-frame numbers for the debug HUD.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    pub fps: f64,
    pub frame_ms: f64,
    pub sim_steps: u32,
    pub sim_dt: f64,
    pub sim_feed_s: f64,
    pub save_us: u64,
}

pub struct Model {
    // Settings: live values, the file baseline, and which fields the CLI holds.
    pub settings: Settings,
    file: Settings,
    overridden: Vec<Overridden>,
    store: Option<Store>,
    saver: Option<saving::Saver>,
    config_notes: Vec<String>,
    save_at: Option<Instant>,

    // The lamp.
    world: World,
    sim_clock: FixedStep,
    pub field: Field,
    speed: SimSpeed,
    /// The speed multiplier in use; eases toward `speed`.
    speed_factor: f64,
    pub style: StyleId,
    pub theme: Theme,
    pub face: &'static dyn Face,
    pub frozen: bool,

    // Clock & pomodoro.
    pub pomodoro: Pomodoro,
    pub local: LocalTime,
    pub flash: Option<Instant>,
    /// First `r` of a reset double press.
    reset_armed: Option<Instant>,
    /// Second `r`: the reset happens once no auto-repeat follows it.
    reset_pending: Option<Instant>,
    /// Latest `r` press, to tell auto-repeat from a double press.
    last_reset_key: Option<Instant>,

    /// The music widget's player, cover and keys.
    pub music: Music,
    /// What the terminal can show pictures with (read once at start;
    /// `pixels` once the terminal confirms it).
    pub caps: Caps,
    /// The pixel protocol the environment promises, being checked with the
    /// terminal (the app writes its query once at start).
    pub probe: Option<Probe>,
    /// Ghostty's config makes cell backgrounds see-through (read once at
    /// start, for `display.cells = "auto"`).
    pub ghostty_translucent: bool,
    /// The cover as a real picture: what the terminal holds and what's on
    /// its way (bytes the loop writes after each frame).
    pub kitty: Kitty,
    /// The cover as an iTerm2 / sixel picture: placed over its cells
    /// (`settle` after each frame is drawn, `write` after its cells).
    pub inline: Inline,
    /// The Spotify library (Web API): login, playlists, likes.
    pub library: Library,
    /// The lyrics widget's lookups and sync.
    pub lyrics: LyricsState,
    /// The widget on the lava `l` moves (`L` picks another).
    pub lava_focus: Option<usize>,

    // Chrome.
    pub overlay: Overlay,
    pub toast: Option<Toast>,
    pub hud: bool,
    pub focused: bool,
    /// The terminal selects text under mouse capture with option held,
    /// not shift (macOS Terminal, iTerm2): help says so. Set by the app.
    pub option_drag: bool,
    pub layout: Layout,
    /// Cell height ÷ width: reported by the terminal, else from config.
    pub cell_aspect: f64,
    /// A cell's width and height in pixels, when the terminal reports
    /// them (sixel pictures are drawn at that size).
    pub cell_px: Option<(u16, u16)>,
    pub stats: FrameStats,
    /// Adaptive quality (§7): reduced grid, then fps, while over budget.
    pub quality: Quality,
    /// The last picker item clicked, for double-clicks.
    last_click: Option<(usize, Instant)>,

    pub now: Instant,
    started: Instant,
    last_tick: Instant,
    /// Effects for the loop to perform, drained each frame.
    pub quit: bool,
    pub bell: bool,
    pub clear: bool,
}

impl Model {
    /// Loads settings from `store` (a problem becomes a toast) and warms
    /// the lamp up. `cell_aspect` is the terminal-reported cell shape.
    pub fn new(
        session: &Session,
        mut store: Store,
        area: Rect,
        cell_aspect: Option<f64>,
        local: LocalTime,
        seed: u64,
        now: Instant,
    ) -> Self {
        let loaded = store.load();
        let (settings, overridden) = session.apply(&loaded.settings);
        let settings = settings.sanitized();
        let sim_clock = FixedStep::new(SIM_HZ);
        let speed = SimSpeed::from_factor(settings.lamp.speed);
        let mut world = World::new(seed, 1.0);
        world.set_heat(settings.lamp.heat);
        let (caps, unconfirmed) = Caps::detect();
        let mut model = Model {
            style: StyleId::by_name(&settings.lamp.style).unwrap_or_default(),
            theme: Theme::new(
                palette_named(&settings.theme.palette),
                depth_for(settings.display.color),
            ),
            face: clock::face_by_name(&settings.clock.face).unwrap_or_else(clock::default_face),
            pomodoro: Pomodoro::new(pomodoro_config(&settings)),
            cell_aspect: cell_aspect.unwrap_or(settings.display.cell_aspect),
            cell_px: None,
            file: loaded.settings,
            overridden,
            store: Some(store),
            saver: None,
            config_notes: Vec::new(),
            save_at: None,
            world,
            sim_clock,
            field: Field::default(),
            speed,
            speed_factor: speed.factor(),
            frozen: false,
            local,
            flash: None,
            reset_armed: None,
            reset_pending: None,
            last_reset_key: None,
            music: Music::default(),
            caps,
            probe: unconfirmed.map(|p| Probe::new(p, now)),
            ghostty_translucent: crate::cells::detect(),
            kitty: Kitty::default(),
            inline: Inline::default(),
            library: Library::new(settings.spotify_client_id()),
            lyrics: LyricsState::default(),
            lava_focus: None,
            overlay: Overlay::None,
            toast: None,
            hud: false,
            focused: true,
            option_drag: false,
            layout: Layout {
                area,
                ..Layout::default()
            },
            stats: FrameStats::default(),
            quality: Quality::default(),
            last_click: None,
            now,
            started: now,
            last_tick: now,
            quit: false,
            bell: false,
            clear: false,
            settings,
        };
        model.warm_up(area, seed);
        if let Some(problem) = loaded.problem {
            model.toast(problem);
        }
        model
    }

    /// Build the world at this window's lamp aspect and run it for a
    /// while, so the first frame shows a lamp that has been going.
    fn warm_up(&mut self, area: Rect, seed: u64) {
        self.sync_music();
        self.relayout(area);
        if let Some(aspect) = self.lamp_aspect() {
            self.world = World::new(seed, aspect);
            self.world.set_heat(self.settings.lamp.heat);
        }
        self.world.prewarm(PREWARM_STEPS, self.sim_clock.dt_secs());
        self.field.prepare(&self.world, 1.0);
    }

    /// Which key set is live.
    pub fn input_mode(&self) -> InputMode {
        match self.overlay {
            Overlay::None if self.music.keys => InputMode::Player,
            Overlay::None => InputMode::Normal,
            Overlay::Help { .. } => InputMode::Help,
            Overlay::Picker(p) => InputMode::Picker {
                opener: p.kind.opener(),
                inline: self.inline_pickers(),
            },
            Overlay::Library(view) => InputMode::Library {
                inline: self.inline_pickers(),
                typing: view.typing,
            },
        }
    }

    /// Tiny/micro terminals get the one-line picker (§4.4).
    pub fn inline_pickers(&self) -> bool {
        SizeTier::of(self.layout.area) <= SizeTier::Tiny
    }

    /// Frame rate to aim for right now (§7: adaptive quality, unfocused
    /// 10). Frozen frames aren't paced at all: see [`Self::idle_until`].
    pub fn target_fps(&self) -> u32 {
        let fps = self.quality.fps(self.settings.display.fps);
        if self.focused {
            fps
        } else {
            fps.min(UNFOCUSED_FPS)
        }
    }

    /// Frozen (§7) with nothing in motion: the next frame isn't due until
    /// what's on screen changes — the clock's minute (or second, when one
    /// is shown), a running pomodoro's second, a pending save. Input wakes
    /// the loop sooner. `None`: pace frames normally.
    pub fn idle_until(&self) -> Option<Instant> {
        let animating =
            self.toast.is_some() || self.flash.is_some() || self.reset_pending.is_some();
        if !self.frozen || animating {
            return None;
        }
        let second = Duration::from_secs(1);
        let into_second = self
            .local
            .wall
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(Duration::ZERO, |d| {
                Duration::from_nanos(d.subsec_nanos().into())
            });
        let seconds_shown = self
            .layout
            .panel
            .iter()
            .chain(&self.layout.on_lava)
            .any(|s| s.items.iter().any(|p| p.form.seconds));
        let to_clock = if seconds_shown {
            second - into_second
        } else {
            let secs = u64::from(59 - self.local.time.second.min(59));
            Duration::from_secs(secs) + second - into_second
        };
        let mut wake = self.now + to_clock;
        if self.pomodoro.status() == Status::Running {
            // The readout rounds up: it changes as `remaining` crosses a
            // whole second.
            let sub = self.pomodoro.remaining(self.now).subsec_nanos();
            let to_tick = if sub == 0 {
                second
            } else {
                Duration::from_nanos(sub.into())
            };
            wake = wake.min(self.now + to_tick);
        }
        if let Some(at) = self.save_at {
            wake = wake.min(at);
        }
        if self.media_on() {
            // The player may change under us (a track ends, someone presses
            // pause in Spotify): look each second.
            wake = wake.min(self.now + second);
        }
        if let Some(at) = self.lyrics.wake(self.now) {
            // The next line (or the end of a fade).
            wake = wake.min(at);
        }
        if let Some(p) = &self.probe {
            // Settled by then, answer or not.
            wake = wake.min(p.deadline());
        }
        if self.kitty.busy() || self.inline.busy() {
            // A cover on its way to the terminal, a slice a frame.
            wake = wake.min(self.now + Duration::from_millis(16));
        }
        if self.saver.as_ref().is_some_and(saving::Saver::busy) || self.library.busy() {
            // Frozen frames still collect save errors and Spotify's
            // answers promptly.
            wake = wake.min(self.now + Duration::from_millis(100));
        }
        Some(wake + WAKE_SLACK)
    }

    /// A frame was drawn: `frame_ms` of work, `dt` after the previous one.
    /// Feeds adaptive quality (not while frozen: idle frames say nothing
    /// about what a moving lamp costs).
    pub fn frame_drawn(&mut self, frame_ms: f64, dt: Duration, now: Instant) {
        self.stats.frame_ms = frame_ms;
        if self.frozen {
            return;
        }
        self.quality.set_workload(self.workload());
        self.quality
            .frame(frame_ms, dt, self.settings.display.fps, now);
    }

    /// Grid samples a frame of the lamp needs at full quality.
    pub fn workload(&self) -> usize {
        self.layout.lamp.map_or(0, |l| {
            let grid = self.style.style().grid();
            usize::from(l.width) * usize::from(grid.x) * usize::from(l.height) * usize::from(grid.y)
        })
    }

    pub fn minimal(&self) -> bool {
        self.settings.minimal()
    }

    pub fn face_options(&self, seconds: bool) -> clock::FaceOptions {
        clock::FaceOptions {
            hour24: self.settings.clock.hour24,
            seconds,
        }
    }

    // --- per frame ---------------------------------------------------------

    /// Advance to `now` and lay out for `area`. Call once per frame, just
    /// before drawing, with the area actually being drawn.
    ///
    /// Sleep: `Instant` stops while the machine is suspended (macOS, Linux),
    /// so a wall clock that jumped ahead of it by [`SLEEP_MIN`] or more is
    /// time spent asleep, and a running pomodoro counts it. The lamp
    /// doesn't: it just carries on from where it was. A wall clock that
    /// jumps *backwards* (NTP, a manual change) is ignored; the pomodoro
    /// only ever moves forward. A manual forward change looks like a sleep
    /// and is treated as one.
    pub fn tick(&mut self, now: Instant, area: Rect, local: LocalTime) {
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        self.now = now;
        if let Some(asleep) = local
            .wall
            .duration_since(self.local.wall)
            .ok()
            .and_then(|wall| wall.checked_sub(elapsed))
            .filter(|&asleep| asleep >= SLEEP_MIN)
        {
            self.pomodoro.slept(asleep);
        }
        self.local = local;

        if let Some(end) = self.pomodoro.tick(now) {
            self.phase_ended(end, now);
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|t| now - t.at >= TOAST_TIME)
        {
            self.toast = None;
        }
        if self.flash.is_some_and(|at| now - at >= FLASH_TIME) {
            self.flash = None;
        }
        if self.reset_armed.is_some_and(|at| now - at >= RESET_WINDOW) {
            self.reset_armed = None;
        }
        if self.reset_pending.is_some_and(|at| now - at >= REPEAT_GAP) {
            self.reset_pending = None;
            self.pomodoro.reset();
            self.toast("pomodoro reset");
        }
        self.poll_saves();
        self.stats.save_us = 0;
        if self.save_at.is_some_and(|at| now >= at) {
            let started = Instant::now();
            self.save();
            self.stats.save_us = started.elapsed().as_micros() as u64;
        }
        if let Some(verdict) = self.probe.as_ref().and_then(|p| p.expired(now)) {
            self.settle_probe(verdict);
        }
        self.sync_music();
        self.sync_library();

        self.relayout(area);
        self.sync_pictures();
        if let Some(aspect) = self.lamp_aspect() {
            self.world.set_aspect(aspect);
        }
        let ease = 1.0 - (-elapsed.as_secs_f64() / SPEED_EASE).exp();
        self.speed_factor += (self.speed.factor() - self.speed_factor) * ease;
        self.stats.sim_steps = 0;
        self.stats.sim_feed_s = 0.0;
        self.stats.sim_dt = self.sim_clock.dt_secs();
        if !self.frozen {
            self.stats.sim_feed_s = elapsed
                .min(FixedStep::STALL)
                .mul_f64(self.speed_factor)
                .as_secs_f64();
            let steps = self.sim_clock.advance(elapsed, self.speed_factor);
            self.stats.sim_steps = steps;
            for _ in 0..steps {
                self.world.step(self.sim_clock.dt_secs());
            }
        }
        self.field.prepare(&self.world, self.sim_clock.alpha());
    }

    /// Recompute the layout for `area` from the current state.
    pub fn relayout(&mut self, area: Rect) {
        self.layout = self.layout_for(area);
        // A taller window shows more help: don't leave its top scrolled away.
        if let Overlay::Help { scroll } = &mut self.overlay {
            *scroll = (*scroll).min(crate::ui::help::sheet::max_scroll(area));
        }
    }

    /// The layout the current state gets at `area` (pure).
    pub fn layout_for(&self, area: Rect) -> Layout {
        let dock: Vec<DockItem> = WIDGETS
            .iter()
            .map(|w| {
                let place = self.settings.dock.place(*w);
                DockItem {
                    place,
                    forms: match place {
                        Place::Off => Vec::new(),
                        _ => w.forms(self, place),
                    },
                    anchor: self.settings.dock.anchor(*w),
                    chip: w.chip(self).map(|c| c.text.width() as u16),
                    rank: w.rank(self),
                }
            })
            .collect();
        let input = LayoutInput {
            minimal: self.minimal(),
            status_bar: self.settings.ui.status_bar,
            dock: &dock,
            cell_aspect: self.cell_aspect,
        };
        layout::layout(area, &input)
    }

    /// The laid-out lamp's visual aspect, which the sim's world follows.
    fn lamp_aspect(&self) -> Option<f64> {
        let lamp = self.layout.lamp?;
        Some(layout::visual_aspect(
            lamp.width,
            lamp.height,
            self.cell_aspect,
        ))
    }

    fn phase_ended(&mut self, end: PhaseEnd, now: Instant) {
        let next = format_remaining(self.pomodoro.config().duration(end.next));
        self.toast(format!("{} · {next}", end.next.label()));
        if !end.skipped {
            self.flash = Some(now);
            self.bell = self.settings.pomodoro.bell;
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            at: self.now,
        });
    }

    /// Settings changed: write them out once things settle.
    fn changed(&mut self, now: Instant) {
        self.save_at = Some(now + SAVE_DELAY);
    }

    /// Write pending settings now (also called on quit).
    pub fn save(&mut self) {
        if self.save_at.take().is_none() {
            return;
        }
        let out = config::to_persist(&self.settings, &self.file, &self.overridden);
        if let Some(saver) = &mut self.saver {
            saver.submit(out);
            return;
        }
        match self.store.as_mut().unwrap().save(&out) {
            Ok(()) => self.file = out,
            Err(problem) => self.toast(problem),
        }
    }

    /// Start once, before entering the terminal loop. Tests keep the inline
    /// Store so pure model tests never depend on thread scheduling.
    pub fn background_saves(&mut self) -> std::io::Result<()> {
        let store = self.store.take().unwrap();
        self.config_notes = store.report();
        self.saver = Some(saving::Saver::start(store)?);
        Ok(())
    }

    fn poll_saves(&mut self) {
        while let Some(saver) = &mut self.saver {
            match saver.poll() {
                Ok(Some((out, Ok(())))) => self.file = out,
                Ok(Some((_, Err(problem)))) | Err(problem) => {
                    self.toast(problem);
                    break;
                }
                Ok(None) => break,
            }
        }
    }

    /// Join only after the loop exits; flush the final change on quit.
    pub fn finish_saves(&mut self) {
        self.save();
        if let Some(saver) = self.saver.take() {
            match saver.finish() {
                Ok((store, results)) => {
                    self.store = Some(store);
                    self.config_notes.clear();
                    for (out, result) in results {
                        match result {
                            Ok(()) => self.file = out,
                            Err(problem) => self.config_notes.push(problem),
                        }
                    }
                }
                Err(problem) => self.config_notes.push(problem),
            }
        }
    }

    /// Config problems to print once the terminal is restored.
    pub fn config_report(&self) -> Vec<String> {
        let mut notes = self.store.as_ref().map_or_else(Vec::new, Store::report);
        notes.extend(self.config_notes.clone());
        notes
    }

    /// The phase-change flash right now: 0 → 1 → 0 over [`FLASH_TIME`].
    pub fn flash_level(&self) -> f32 {
        self.flash.map_or(0.0, |at| {
            let t = self.now.duration_since(at).as_secs_f32() / FLASH_TIME.as_secs_f32();
            (t * std::f32::consts::PI).sin().max(0.0)
        })
    }

    /// Seconds since launch.
    pub fn time(&self) -> f64 {
        self.now.duration_since(self.started).as_secs_f64()
    }
}

/// `heat ▮▮▮▯▯`.
pub fn heat_toast(heat: u8) -> String {
    let filled = usize::from(heat);
    let levels = usize::from(*HEAT_LEVELS.end());
    format!("heat {}{}", "▮".repeat(filled), "▯".repeat(levels - filled))
}

/// `speed ×2`, `speed ×0.25`.
pub fn speed_toast(speed: SimSpeed) -> String {
    format!("speed ×{}", speed.factor())
}

impl Model {
    /// Whether the lamp draws for see-through cell backgrounds
    /// (`display.cells`; see `render::cell::half_block`).
    pub fn translucent_cells(&self) -> bool {
        match self.settings.display.cells {
            CellsChoice::Auto => self.ghostty_translucent,
            CellsChoice::Opaque => false,
            CellsChoice::Translucent => true,
        }
    }
}

fn palette_named(name: &str) -> &'static Palette {
    Palette::by_name(name).unwrap_or(&Palette::all()[0])
}

fn depth_for(choice: ColorChoice) -> ColorDepth {
    match choice {
        ColorChoice::Auto => ColorDepth::detect(),
        ColorChoice::Truecolor => ColorDepth::TrueColor,
        ColorChoice::Ansi256 => ColorDepth::Ansi256,
        ColorChoice::Ansi16 => ColorDepth::Ansi16,
        ColorChoice::None => ColorDepth::None,
    }
}

fn pomodoro_config(settings: &Settings) -> PomodoroConfig {
    let p = &settings.pomodoro;
    let min = |m: u32| Duration::from_secs(u64::from(m) * 60);
    PomodoroConfig {
        focus: min(p.focus_min),
        short_break: min(p.short_break_min),
        long_break: min(p.long_break_min),
        cycles: p.cycles,
        auto_advance: true,
    }
}

#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod tests;
