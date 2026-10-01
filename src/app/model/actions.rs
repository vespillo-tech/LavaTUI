//! [`Model::update`]: what each [`Action`] does. An open overlay gets the
//! first look; [`Model::global_action`] maps the rest, one arm per action,
//! onto small helpers.

use std::time::Instant;

use super::{Model, Overlay, PickerKind, REPEAT_GAP, heat_toast, speed_toast};
use crate::clock::{self, Status, format_remaining};
use crate::config::{Overridden, Settings, UiMode};
use crate::sim::{DEFAULT_HEAT, SimSpeed};
use crate::ui::keymap::Action;
use crate::ui::layout::LampFrame;

/// Mixed into the reseed time (the 64-bit golden ratio, as in SplitMix64),
/// so presses close together still get far-apart seeds.
const RESEED_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Toasts for the boolean settings' new value: `[on, off]`.
const STATUS_BAR: [&str; 2] = ["status bar on", "status bar off"];
const LIGHTING: [&str; 2] = ["lighting on", "lighting off"];
const CLOCK: [&str; 2] = ["clock shown", "clock hidden"];
const HOUR24: [&str; 2] = ["24h", "12h"];

impl Model {
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

    fn help_action(&mut self, scroll: u16, action: Action) -> bool {
        match action {
            Action::Up => {
                self.overlay = Overlay::Help {
                    scroll: scroll.saturating_sub(1),
                }
            }
            Action::Down => {
                self.overlay = Overlay::Help {
                    scroll: (scroll + 1).min(crate::ui::help::sheet::max_scroll(self.layout.area)),
                }
            }
            Action::Close => self.overlay = Overlay::None,
            _ => return false,
        }
        true
    }

    /// What an action does with no overlay open (or one it passes).
    fn global_action(&mut self, action: Action, now: Instant) {
        match action {
            Action::Quit => self.quit = true,
            Action::Help => self.overlay = Overlay::Help { scroll: 0 },
            // Overlay keys, with no overlay open.
            Action::Close
            | Action::Up
            | Action::Down
            | Action::Keep
            | Action::Jump(_)
            | Action::Click { .. } => {}
            Action::ToggleMinimal => self.toggle_minimal(now),
            Action::ToggleStatusBar if !self.minimal() => {
                self.toggle(now, |s| &mut s.ui.status_bar, STATUS_BAR);
            }
            Action::ToggleStatusBar => {}
            Action::NextStyle => self.cycle_pick(PickerKind::Style),
            Action::NextFace => {
                self.face = clock::next_face(self.face.name());
                self.persist_pick(PickerKind::Face);
                self.toast_cycle(PickerKind::Face);
            }
            Action::NextPalette => self.cycle_pick(PickerKind::Palette),
            Action::StylePicker => self.open_picker(PickerKind::Style),
            Action::FacePicker => self.open_picker(PickerKind::Face),
            Action::PalettePicker => self.open_picker(PickerKind::Palette),
            Action::CycleFrame => self.cycle_frame(now),
            Action::ToggleLighting => self.toggle(now, |s| &mut s.lamp.lighting, LIGHTING),
            Action::ToggleClock => self.toggle(now, |s| &mut s.clock.show, CLOCK),
            Action::ToggleHour24 => self.toggle(now, |s| &mut s.clock.hour24, HOUR24),
            Action::PomodoroToggle => self.pomodoro_toggle(now),
            Action::PomodoroSkip => match self.pomodoro.skip(now) {
                Some(end) => self.phase_ended(end, now),
                None => self.toast("pomodoro idle · ␣ to start"),
            },
            Action::PomodoroReset => self.reset_key(now),
            Action::HeatUp => self.set_heat(self.world.heat().saturating_add(1), now),
            Action::HeatDown => self.set_heat(self.world.heat().saturating_sub(1), now),
            Action::Faster => self.set_speed(self.speed.faster(), now),
            Action::Slower => self.set_speed(self.speed.slower(), now),
            Action::ResetHeatSpeed => self.reset_heat_speed(now),
            Action::Freeze => {
                self.frozen = !self.frozen;
                self.toast(if self.frozen { "frozen" } else { "thawed" });
            }
            Action::Reseed => {
                let seed = now.duration_since(self.started).as_nanos() as u64 ^ RESEED_MIX;
                self.world.reseed(seed);
                self.toast("reseeding");
            }
            Action::DebugHud => self.hud = !self.hud,
            // A resize needs no clear of its own: the terminal clears and
            // repaints in full whenever the size it draws at changes.
            Action::Redraw => self.clear = true,
            Action::Resize => {}
            Action::Focus(focused) => self.focused = focused,
            Action::Poke { col, row } => self.poke(col, row),
        }
    }

    /// Flip the setting `flag` points at, toast `[on, off]` for its new
    /// value and schedule a save.
    fn toggle(
        &mut self,
        now: Instant,
        flag: fn(&mut Settings) -> &mut bool,
        [on, off]: [&'static str; 2],
    ) {
        let value = flag(&mut self.settings);
        *value = !*value;
        let text = if *value { on } else { off };
        self.toast(text);
        self.changed(now);
    }

    fn toggle_minimal(&mut self, now: Instant) {
        let mode = &mut self.settings.ui.mode;
        *mode = match mode {
            UiMode::Full => UiMode::Minimal,
            UiMode::Minimal => UiMode::Full,
        };
        self.overridden.retain(|o| *o != Overridden::Mode);
        if self.minimal() {
            self.toast("minimal · m to return");
        }
        self.changed(now);
    }

    /// Next frame mode; the toast says what auto resolved to.
    fn cycle_frame(&mut self, now: Instant) {
        let frame = &mut self.settings.lamp.frame;
        *frame = frame.next();
        self.relayout(self.layout.area);
        let resolved = match self.layout.lamp.map(|l| l.frame) {
            Some(LampFrame::Glass) => "glass",
            _ => "bleed",
        };
        self.toast(format!("{} · {resolved}", self.settings.lamp.frame.name()));
        self.changed(now);
    }

    /// The next style or palette, kept (no preview).
    fn cycle_pick(&mut self, kind: PickerKind) {
        let n = kind.items().len();
        self.apply_pick(kind, (self.current(kind) + 1) % n);
        self.persist_pick(kind);
        self.toast_cycle(kind);
    }

    fn set_heat(&mut self, heat: u8, now: Instant) {
        self.world.set_heat(heat);
        self.settings.lamp.heat = self.world.heat();
        self.toast(heat_toast(self.world.heat()));
        self.changed(now);
    }

    fn reset_heat_speed(&mut self, now: Instant) {
        self.world.set_heat(DEFAULT_HEAT);
        self.speed = SimSpeed::default();
        self.settings.lamp.heat = DEFAULT_HEAT;
        self.settings.lamp.speed = self.speed.factor();
        let (heat, speed) = (heat_toast(DEFAULT_HEAT), speed_toast(self.speed));
        self.toast(format!("{heat} · {speed}"));
        self.changed(now);
    }

    fn set_speed(&mut self, speed: SimSpeed, now: Instant) {
        self.speed = speed;
        self.settings.lamp.speed = speed.factor();
        self.toast(speed_toast(speed));
        self.changed(now);
    }

    fn pomodoro_toggle(&mut self, now: Instant) {
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

    /// `r`: reset the pomodoro on a double press.
    ///
    /// Terminals without key-release reports send a held key as a stream
    /// of presses. The second press only resets once `tick` sees no
    /// auto-repeat follow it, so holding r never resets (terminals that do
    /// report repeats are filtered in the keymap).
    fn reset_key(&mut self, now: Instant) {
        let repeat = self
            .last_reset_key
            .replace(now)
            .is_some_and(|last| now - last < REPEAT_GAP);
        if repeat {
            self.reset_armed = None;
            self.reset_pending = None;
        } else if self.reset_pending.is_none() {
            if self.reset_armed.take().is_some() {
                self.reset_pending = Some(now);
            } else {
                self.reset_armed = Some(now);
                self.toast("press r again to reset");
            }
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
}
