//! The app's state and everything that changes it.
//!
//! [`Model::update`] applies one [`Action`] (a key, the mouse, a resize);
//! [`Model::tick`] advances time once per frame (pomodoro, toasts, sim
//! steps, debounced config save) and recomputes the layout. Neither touches
//! the terminal: time and the local clock are passed in, and the side
//! effects the loop must perform (bell, full clear) are flags it drains.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::clock::{
    self, ClockTime, Face, PhaseEnd, Pomodoro, PomodoroConfig, Status, format_remaining,
};
use crate::config::store::Store;
use crate::config::{self, ColorChoice, Overridden, Session, Settings, UiMode};
use crate::render::StyleId;
use crate::sim::{DEFAULT_HEAT, Field, HEAT_LEVELS, Shape, SimSpeed, World};
use crate::theme::{ColorDepth, Palette, Theme};
use crate::timing::FixedStep;
use crate::ui::keymap::{Action, InputMode};
use crate::ui::layout::{self, ChipKind, LampFrame, Layout, LayoutInput, SizeTier};

/// Simulation rate. Fixed; unrelated to the render frame rate.
pub const SIM_HZ: u32 = 120;
/// Headless steps run at launch so the first frame already looks alive.
const PREWARM_STEPS: u32 = 600;
/// How long a toast stays up (§4.2).
pub const TOAST_TIME: Duration = Duration::from_millis(1400);
/// Phase-change flash length (§4.5).
pub const FLASH_TIME: Duration = Duration::from_millis(600);
/// `r` must be pressed twice within this to reset the pomodoro.
const RESET_WINDOW: Duration = Duration::from_secs(2);
/// Settings are written this long after the last change.
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// Speed changes ease in with this time constant (~95 % in 0.5 s).
const SPEED_EASE: f64 = 0.17;
/// Frame rates when nothing needs 60 fps (§7).
const UNFOCUSED_FPS: u32 = 10;
const FROZEN_FPS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Style,
    Face,
    Palette,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            PickerKind::Style => "style",
            PickerKind::Face => "clock",
            PickerKind::Palette => "palette",
        }
    }

    pub fn items(self) -> Vec<&'static str> {
        match self {
            PickerKind::Style => StyleId::all().map(|s| s.style().name()).collect(),
            PickerKind::Face => clock::FACES.iter().map(|f| f.name()).collect(),
            PickerKind::Palette => Palette::all().iter().map(|p| p.name).collect(),
        }
    }

    fn opener(self) -> Action {
        match self {
            PickerKind::Style => Action::StylePicker,
            PickerKind::Face => Action::FacePicker,
            PickerKind::Palette => Action::PalettePicker,
        }
    }
}

/// An open picker: the cursor previews live; `original` is what esc restores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    pub cursor: usize,
    pub original: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help { scroll: u16 },
    Picker(Picker),
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
}

/// Measured per-frame numbers for the debug HUD.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    pub fps: f64,
    pub frame_ms: f64,
}

pub struct Model {
    // Settings: live values, the file baseline, and which fields the CLI holds.
    pub settings: Settings,
    file: Settings,
    overridden: Vec<Overridden>,
    store: Store,
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
    reset_armed: Option<Instant>,

    // Chrome.
    pub overlay: Overlay,
    pub toast: Option<Toast>,
    pub hud: bool,
    pub focused: bool,
    pub layout: Layout,
    /// Cell height ÷ width: reported by the terminal, else from config.
    pub cell_aspect: f64,
    pub stats: FrameStats,

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
        let mut world = World::new(seed, 1.0, Shape::Tank);
        world.set_heat(settings.lamp.heat);
        let mut model = Model {
            style: StyleId::by_name(&settings.lamp.style).unwrap_or_default(),
            theme: Theme::new(
                palette_named(&settings.theme.palette),
                depth_for(settings.display.color),
            ),
            face: clock::face_by_name(&settings.clock.face).unwrap_or_else(clock::default_face),
            pomodoro: Pomodoro::new(pomodoro_config(&settings)),
            cell_aspect: cell_aspect.unwrap_or(settings.display.cell_aspect),
            file: loaded.settings,
            overridden,
            store,
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
            overlay: Overlay::None,
            toast: None,
            hud: false,
            focused: true,
            layout: Layout {
                area,
                lamp: None,
                status: None,
                panel: None,
                chip: None,
                toast: None,
            },
            stats: FrameStats::default(),
            now,
            started: now,
            last_tick: now,
            quit: false,
            bell: false,
            clear: false,
            settings,
        };
        // Build the world in this window's container before warming up, so
        // the first frame shows a lamp that has been running in this shape.
        model.relayout(area);
        if let Some(lamp) = model.layout.lamp {
            let aspect =
                layout::visual_aspect(lamp.view.width, lamp.view.height, model.cell_aspect);
            model.world = World::new(seed, aspect, shape_for(lamp.frame));
            model.world.set_heat(model.settings.lamp.heat);
        }
        model
            .world
            .prewarm(PREWARM_STEPS, model.sim_clock.dt_secs());
        model.field.prepare(&model.world, 1.0);
        if let Some(problem) = loaded.problem {
            model.toast(problem);
        }
        model
    }

    /// Which key set is live.
    pub fn input_mode(&self) -> InputMode {
        match self.overlay {
            Overlay::None => InputMode::Normal,
            Overlay::Help { .. } => InputMode::Help,
            Overlay::Picker(p) => InputMode::Picker {
                opener: p.kind.opener(),
                inline: self.inline_pickers(),
            },
        }
    }

    /// Tiny/micro terminals get the one-line picker (§4.4).
    pub fn inline_pickers(&self) -> bool {
        SizeTier::of(self.layout.area) <= SizeTier::Tiny
    }

    /// Frame rate to aim for right now (§7: unfocused 10, frozen low).
    pub fn target_fps(&self) -> u32 {
        let fps = self.settings.display.fps;
        if self.frozen {
            fps.min(FROZEN_FPS)
        } else if !self.focused {
            fps.min(UNFOCUSED_FPS)
        } else {
            fps
        }
    }

    pub fn minimal(&self) -> bool {
        self.settings.minimal()
    }

    /// What a chip would show right now, and its text.
    pub fn chip_text(&self) -> Option<(ChipKind, String)> {
        let glyph = match self.pomodoro.status() {
            Status::Running => '▸',
            Status::Paused => '‖',
            Status::Idle if self.settings.clock.show => {
                let hour24 = self.settings.clock.hour24;
                let text = clock::readout(self.local.time, hour24, false, !hour24);
                return Some((ChipKind::Clock, text));
            }
            Status::Idle => return None,
        };
        let remaining = format_remaining(self.pomodoro.remaining(self.now));
        Some((ChipKind::Pomodoro, format!("{glyph} {remaining}")))
    }

    pub fn face_options(&self, seconds: bool) -> clock::FaceOptions {
        clock::FaceOptions {
            hour24: self.settings.clock.hour24,
            seconds,
        }
    }

    // --- per frame ---------------------------------------------------------

    /// Advance to `now` and lay out for `area`. Call once per frame, just
    /// before drawing.
    pub fn tick(&mut self, now: Instant, area: Rect, local: LocalTime) {
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        self.now = now;
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
        if self.save_at.is_some_and(|at| now >= at) {
            self.save();
        }

        self.relayout(area);
        self.sync_world_shape();
        let ease = 1.0 - (-elapsed.as_secs_f64() / SPEED_EASE).exp();
        self.speed_factor += (self.speed.factor() - self.speed_factor) * ease;
        if !self.frozen {
            let steps = self.sim_clock.advance(elapsed.mul_f64(self.speed_factor));
            for _ in 0..steps {
                self.world.step(self.sim_clock.dt_secs());
            }
        }
        self.field.prepare(&self.world, self.sim_clock.alpha());
    }

    /// Recompute the layout for `area` from the current state.
    pub fn relayout(&mut self, area: Rect) {
        let chip = self.chip_text();
        let input = LayoutInput {
            minimal: self.minimal(),
            status_bar: self.settings.ui.status_bar,
            frame: self.settings.lamp.frame,
            show_clock: self.settings.clock.show,
            face: self.face,
            hour24: self.settings.clock.hour24,
            chip: chip.map(|(kind, text)| (kind, text.chars().count() as u16)),
            minimal_clock: self.settings.minimal.clock,
            cell_aspect: self.cell_aspect,
        };
        self.layout = layout::layout(area, &input);
    }

    /// Match the sim's container to the layout's lamp.
    fn sync_world_shape(&mut self) {
        let Some(lamp) = self.layout.lamp else {
            return;
        };
        let shape = shape_for(lamp.frame);
        let aspect = layout::visual_aspect(lamp.view.width, lamp.view.height, self.cell_aspect);
        self.world.set_shape(shape, aspect);
    }

    fn phase_ended(&mut self, end: PhaseEnd, now: Instant) {
        let next = format_remaining(self.pomodoro.config().duration(end.next));
        self.toast(format!("{} · {next}", end.next.label()));
        if !end.skipped {
            self.flash = Some(now);
            self.bell = self.settings.pomodoro.bell;
        }
    }

    // --- actions -----------------------------------------------------------

    pub fn update(&mut self, action: Action, now: Instant) {
        self.now = now;
        let handled = match self.overlay {
            Overlay::Picker(picker) => self.picker_action(picker, action),
            Overlay::Help { scroll } => self.help_action(scroll, action),
            Overlay::None => false,
        };
        // Under an overlay only quitting and terminal events get through (§6.2).
        let passes = self.overlay == Overlay::None
            || matches!(
                action,
                Action::Quit | Action::Resize | Action::Focus(_) | Action::Redraw
            );
        if !handled && passes {
            self.global_action(action, now);
        }
        self.relayout(self.layout.area);
    }

    /// Returns whether the action was a picker action.
    fn picker_action(&mut self, mut picker: Picker, action: Action) -> bool {
        let n = picker.kind.items().len();
        // The opening key again keeps and closes.
        let action = if action == picker.kind.opener() {
            Action::Keep
        } else {
            action
        };
        match action {
            Action::Up => picker.cursor = (picker.cursor + n - 1) % n,
            Action::Down => picker.cursor = (picker.cursor + 1) % n,
            Action::Jump(i) if usize::from(i) < n => picker.cursor = usize::from(i),
            Action::Jump(_) => return true,
            Action::Keep => {
                self.overlay = Overlay::None;
                self.persist_pick(picker.kind);
                return true;
            }
            Action::Close => {
                self.apply_pick(picker.kind, picker.original);
                self.overlay = Overlay::None;
                return true;
            }
            _ => return false,
        }
        self.apply_pick(picker.kind, picker.cursor);
        self.overlay = Overlay::Picker(picker);
        true
    }

    fn help_action(&mut self, scroll: u16, action: Action) -> bool {
        match action {
            Action::Up => {
                self.overlay = Overlay::Help {
                    scroll: scroll.saturating_sub(1),
                }
            }
            Action::Down => {
                self.overlay = Overlay::Help {
                    scroll: (scroll + 1).min(crate::ui::help::max_scroll(self.layout.area)),
                }
            }
            Action::Close => self.overlay = Overlay::None,
            _ => return false,
        }
        true
    }

    fn global_action(&mut self, action: Action, now: Instant) {
        let s = &mut self.settings;
        match action {
            Action::Quit => self.quit = true,
            Action::Help => self.overlay = Overlay::Help { scroll: 0 },
            Action::Close | Action::Up | Action::Down | Action::Keep | Action::Jump(_) => {}
            Action::ToggleMinimal => {
                s.ui.mode = match s.ui.mode {
                    UiMode::Full => UiMode::Minimal,
                    UiMode::Minimal => UiMode::Full,
                };
                self.overridden.retain(|o| *o != Overridden::Mode);
                if self.minimal() {
                    self.toast("minimal · m to return");
                }
                self.changed(now);
            }
            Action::ToggleStatusBar if !s.minimal() => {
                s.ui.status_bar = !s.ui.status_bar;
                let text = if s.ui.status_bar {
                    "status bar on"
                } else {
                    "status bar off"
                };
                self.toast(text);
                self.changed(now);
            }
            Action::ToggleStatusBar => {}
            Action::NextStyle => {
                self.apply_pick(PickerKind::Style, self.style.next().index());
                self.persist_pick(PickerKind::Style);
                self.toast_cycle(PickerKind::Style);
            }
            Action::NextFace => {
                self.face = clock::next_face(self.face.name());
                self.persist_pick(PickerKind::Face);
                self.toast_cycle(PickerKind::Face);
            }
            Action::NextPalette => {
                let i = self.current(PickerKind::Palette);
                self.apply_pick(PickerKind::Palette, (i + 1) % Palette::all().len());
                self.persist_pick(PickerKind::Palette);
                self.toast_cycle(PickerKind::Palette);
            }
            Action::StylePicker => self.open_picker(PickerKind::Style),
            Action::FacePicker => self.open_picker(PickerKind::Face),
            Action::PalettePicker => self.open_picker(PickerKind::Palette),
            Action::CycleFrame => {
                s.lamp.frame = s.lamp.frame.next();
                self.relayout(self.layout.area);
                let resolved = match self.layout.lamp.map(|l| l.frame) {
                    Some(LampFrame::Glass) => "glass",
                    _ => "bleed",
                };
                self.toast(format!("{} · {resolved}", self.settings.lamp.frame.name()));
                self.changed(now);
            }
            Action::ToggleLighting => {
                s.lamp.lighting = !s.lamp.lighting;
                let text = if s.lamp.lighting {
                    "lighting on"
                } else {
                    "lighting off"
                };
                self.toast(text);
                self.changed(now);
            }
            Action::ToggleClock => {
                s.clock.show = !s.clock.show;
                let text = if s.clock.show {
                    "clock shown"
                } else {
                    "clock hidden"
                };
                self.toast(text);
                self.changed(now);
            }
            Action::ToggleHour24 => {
                s.clock.hour24 = !s.clock.hour24;
                let text = if s.clock.hour24 { "24h" } else { "12h" };
                self.toast(text);
                self.changed(now);
            }
            Action::PomodoroToggle => {
                let was = self.pomodoro.status();
                self.pomodoro.toggle(now);
                let text = match (was, self.pomodoro.status()) {
                    (Status::Idle, _) => format!(
                        "{} · {}",
                        self.pomodoro.phase().label(),
                        format_remaining(self.pomodoro.remaining(now))
                    ),
                    (_, Status::Paused) => "paused".into(),
                    _ => "resumed".into(),
                };
                self.toast(text);
            }
            Action::PomodoroSkip => match self.pomodoro.skip(now) {
                Some(end) => self.phase_ended(end, now),
                None => self.toast("pomodoro idle · ␣ to start"),
            },
            Action::PomodoroReset => {
                if self.reset_armed.take().is_some() {
                    self.pomodoro.reset();
                    self.toast("pomodoro reset");
                } else {
                    self.reset_armed = Some(now);
                    self.toast("press r again to reset");
                }
            }
            Action::HeatDown | Action::HeatUp => {
                let heat = self.world.heat();
                let heat = if action == Action::HeatUp {
                    heat.saturating_add(1)
                } else {
                    heat.saturating_sub(1)
                };
                self.world.set_heat(heat);
                self.settings.lamp.heat = self.world.heat();
                self.toast(heat_toast(self.world.heat()));
                self.changed(now);
            }
            Action::Slower | Action::Faster => {
                self.speed = if action == Action::Faster {
                    self.speed.faster()
                } else {
                    self.speed.slower()
                };
                self.settings.lamp.speed = self.speed.factor();
                self.toast(speed_toast(self.speed));
                self.changed(now);
            }
            Action::ResetHeatSpeed => {
                self.world.set_heat(DEFAULT_HEAT);
                self.speed = SimSpeed::default();
                self.settings.lamp.heat = DEFAULT_HEAT;
                self.settings.lamp.speed = self.speed.factor();
                self.toast(format!(
                    "{} · {}",
                    heat_toast(DEFAULT_HEAT),
                    speed_toast(self.speed)
                ));
                self.changed(now);
            }
            Action::Freeze => {
                self.frozen = !self.frozen;
                self.toast(if self.frozen { "frozen" } else { "thawed" });
            }
            Action::Reseed => {
                let seed =
                    now.duration_since(self.started).as_nanos() as u64 ^ 0x9E37_79B9_7F4A_7C15;
                self.world.reseed(seed);
                self.toast("reseeding");
            }
            Action::DebugHud => self.hud = !self.hud,
            Action::Redraw => self.clear = true,
            Action::Resize => self.clear = true,
            Action::Focus(focused) => self.focused = focused,
            Action::Poke { col, row } => self.poke(col, row),
        }
    }

    /// A mouse click on the wax: warm it there.
    fn poke(&mut self, col: u16, row: u16) {
        let Some(lamp) = self.layout.lamp else {
            return;
        };
        let view = lamp.view;
        if !view.contains((col, row).into()) {
            return;
        }
        let u = (f64::from(col - view.x) + 0.5) / f64::from(view.width);
        let v = (f64::from(row - view.y) + 0.5) / f64::from(view.height);
        self.world.heat_pulse(u, v);
    }

    fn open_picker(&mut self, kind: PickerKind) {
        let i = self.current(kind);
        self.overlay = Overlay::Picker(Picker {
            kind,
            cursor: i,
            original: i,
        });
    }

    /// Index of the active item of a picker's kind.
    pub fn current(&self, kind: PickerKind) -> usize {
        match kind {
            PickerKind::Style => self.style.index(),
            PickerKind::Face => clock::FACES
                .iter()
                .position(|f| f.name() == self.face.name())
                .unwrap_or(0),
            PickerKind::Palette => Palette::all()
                .iter()
                .position(|p| p.name == self.theme.palette().name)
                .unwrap_or(0),
        }
    }

    /// Make item `i` live (preview; not yet saved).
    fn apply_pick(&mut self, kind: PickerKind, i: usize) {
        match kind {
            PickerKind::Style => {
                if let Some(id) = StyleId::all().nth(i) {
                    self.style = id;
                }
            }
            PickerKind::Face => {
                if let Some(face) = clock::FACES.get(i) {
                    self.face = *face;
                }
            }
            PickerKind::Palette => {
                if let Some(palette) = Palette::all().get(i) {
                    self.theme = Theme::new(palette, self.theme.depth());
                }
            }
        }
    }

    /// Record the live item of `kind` in the settings and schedule a save.
    fn persist_pick(&mut self, kind: PickerKind) {
        let now = self.now;
        match kind {
            PickerKind::Style => {
                self.settings.lamp.style = self.style.style().name().into();
                self.overridden.retain(|o| *o != Overridden::Style);
            }
            PickerKind::Face => self.settings.clock.face = self.face.name().into(),
            PickerKind::Palette => {
                self.settings.theme.palette = self.theme.palette().name.into();
                self.overridden.retain(|o| *o != Overridden::Palette);
            }
        }
        self.changed(now);
    }

    fn toast_cycle(&mut self, kind: PickerKind) {
        let items = kind.items();
        let i = self.current(kind);
        self.toast(format!("{}  {}/{}", items[i], i + 1, items.len()));
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
        match self.store.save(&out) {
            Ok(()) => self.file = out,
            Err(_) => self.toast("couldn't save settings"),
        }
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

fn shape_for(frame: LampFrame) -> Shape {
    match frame {
        LampFrame::Glass => Shape::Bottle,
        LampFrame::Bleed => Shape::Tank,
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
mod tests;
