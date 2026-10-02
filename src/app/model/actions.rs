//! [`Model::update`]: what each [`Action`] does. An open overlay gets the
//! first look; [`Model::global_action`] maps the rest, one arm per action,
//! onto small helpers.

use std::time::Instant;

use super::{Model, Overlay, PickerKind, REPEAT_GAP, heat_toast, speed_toast};
use crate::clock::{self, Status, format_remaining};
use crate::config::{Overridden, Settings, UiMode};
use crate::dock::{self, Place};
use crate::sim::{DEFAULT_HEAT, SimSpeed};
use crate::ui::keymap::Action;

/// Mixed into the reseed time (the 64-bit golden ratio, as in SplitMix64),
/// so presses close together still get far-apart seeds.
const RESEED_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Toasts for the boolean settings' new value: `[on, off]`.
const STATUS_BAR: [&str; 2] = ["status bar on", "status bar off"];
const HOUR24: [&str; 2] = ["24h", "12h"];

impl Model {
    pub fn update(&mut self, action: Action, now: Instant) {
        self.now = now;
        let handled = match self.overlay {
            Overlay::Picker(picker) => self.picker_action(picker, action),
            Overlay::Help { scroll } => self.help_action(scroll, action),
            Overlay::Library(view) => self.library_action(view, action),
            Overlay::Settings(view) => self.settings_action(view, action),
            Overlay::None if self.music.keys => self.player_action(action, now),
            Overlay::None => false,
        };
        // Under an overlay (or in the player keys) only quitting and
        // terminal events get through (§6.2).
        let passes = (self.overlay == Overlay::None && !self.music.keys)
            || matches!(
                action,
                Action::Quit | Action::Resize | Action::Focus(_) | Action::Redraw
            );
        if !handled && passes {
            self.global_action(action, now);
        }
        self.sync_music();
        self.sync_library();
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
            Action::Settings => self.open_settings(),
            _ => return false,
        }
        true
    }

    /// The player keys (`A`): each sends its command; `esc` / `q` / `A`
    /// leave them, `?` leaves them for the help.
    fn player_action(&mut self, action: Action, now: Instant) -> bool {
        match action {
            Action::Player(key) => self.player_key(key, now),
            Action::Press { col, row } => self.press(col, row, now),
            Action::Close | Action::PlayerKeys => self.music.keys = false,
            Action::Help => {
                self.music.keys = false;
                self.overlay = Overlay::Help { scroll: 0 };
            }
            _ => return false,
        }
        true
    }

    /// What an action does with no overlay open (or one it passes).
    fn global_action(&mut self, action: Action, now: Instant) {
        match action {
            Action::Quit => self.quit = true,
            Action::Help => self.overlay = Overlay::Help { scroll: 0 },
            Action::Settings => self.open_settings(),
            // Overlay keys, with no overlay open.
            Action::Close
            | Action::Up
            | Action::Down
            | Action::Keep
            | Action::Jump(_)
            | Action::Page(_)
            | Action::Edge(_)
            | Action::Back
            | Action::PlayAll
            | Action::Change(_)
            | Action::SwitchPage(_)
            | Action::Find
            | Action::Type(_)
            | Action::Erase
            | Action::ClearFind
            | Action::Click { .. } => {}
            Action::ToggleMinimal => self.toggle_minimal(now),
            Action::ToggleStatusBar if !self.minimal() => {
                self.toggle(now, |s| &mut s.ui.status_bar, STATUS_BAR);
            }
            Action::ToggleStatusBar => self.toast("no status bar in minimal · m to leave"),
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
            Action::Place(name) => self.move_widget(name, now),
            Action::NextAnchor => self.next_anchor(now),
            Action::NextLavaWidget => self.next_lava_widget(),
            Action::PlayerKeys => self.player_keys_on(),
            Action::CoverDetail => self.next_cover_detail(now),
            // Only while the player keys are on (`player_action`).
            Action::Player(_) => {}
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
            Action::Press { col, row } => self.press(col, row, now),
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

    /// A widget's key: side → lava → off → side. The toast says where it
    /// went, and when it has no room there.
    fn move_widget(&mut self, name: &str, now: Instant) {
        let Some((index, widget)) = dock::by_name(name) else {
            return;
        };
        let place = self.settings.dock.place(widget).next();
        self.settings.dock.set(widget, place);
        self.changed(now);
        self.sync_music();
        self.relayout(self.layout.area);
        if place == Place::Overlay {
            self.lava_focus = Some(index);
        }
        let note = match place {
            Place::Off => "",
            _ if self.layout.placed(index).is_some() => "",
            Place::Side if self.minimal() => " · not in minimal",
            _ => " · no room",
        };
        // Placing the lyrics is the opt-in to lookups: say where they go.
        let via = if name == "lyrics" && place != Place::Off {
            " · via lrclib.net"
        } else {
            ""
        };
        self.toast(format!("{name} · {}{note}{via}", place.describe()));
    }

    /// The widgets on the lava, by index.
    fn on_the_lava(&self) -> Vec<usize> {
        (0..dock::WIDGETS.len())
            .filter(|&i| self.settings.dock.place(dock::WIDGETS[i]) == Place::Overlay)
            .collect()
    }

    /// The widget `l` moves: the one last put on the lava or picked with
    /// `L`, else the first there.
    fn focused_lava_widget(&self) -> Option<usize> {
        let there = self.on_the_lava();
        self.lava_focus
            .filter(|i| there.contains(i))
            .or_else(|| there.first().copied())
    }

    /// `l`: the focused lava widget moves round: centre, then the edge
    /// clockwise. The toast names it and where it went.
    fn next_anchor(&mut self, now: Instant) {
        let Some(index) = self.focused_lava_widget() else {
            self.toast("nothing on the lava · t f a put widgets there");
            return;
        };
        let widget = dock::WIDGETS[index];
        let anchor = self.settings.dock.anchor(widget).next();
        self.settings.dock.set_anchor(widget, anchor);
        self.lava_focus = Some(index);
        self.changed(now);
        self.relayout(self.layout.area);
        let note = if self.layout.placed(index).is_some() {
            ""
        } else {
            " · no room"
        };
        self.toast(format!("{} · {}{note}", widget.name(), anchor.name()));
    }

    /// `L`: `l` moves the next widget on the lava.
    fn next_lava_widget(&mut self) {
        let there = self.on_the_lava();
        let Some(current) = self.focused_lava_widget() else {
            self.toast("nothing on the lava · t f a put widgets there");
            return;
        };
        let at = there.iter().position(|&i| i == current).unwrap_or(0);
        let next = there[(at + 1) % there.len()];
        self.lava_focus = Some(next);
        let widget = dock::WIDGETS[next];
        let anchor = self.settings.dock.anchor(widget).name();
        self.toast(format!("l moves {} · now {anchor}", widget.name()));
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

    /// A mouse press: a music widget's button or bar, else the wax.
    fn press(&mut self, col: u16, row: u16, now: Instant) {
        match self.music_hit(col, row) {
            Some(key) => self.player_key(key, now),
            None => self.poke(col, row),
        }
    }

    /// A mouse click on the wax: warm it there.
    fn poke(&mut self, col: u16, row: u16) {
        let Some(view) = self.layout.lamp else {
            return;
        };
        if !view.contains((col, row).into()) {
            return;
        }
        let u = (f64::from(col - view.x) + 0.5) / f64::from(view.width);
        let v = (f64::from(row - view.y) + 0.5) / f64::from(view.height);
        self.world.heat_pulse(u, v);
    }
}
