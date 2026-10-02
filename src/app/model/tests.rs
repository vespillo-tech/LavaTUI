use std::path::PathBuf;

use super::*;
use crate::dock::{Anchor, Clock, DockWidget, Place, Pomodoro};
use crate::ui::keymap::Action;
use crate::ui::picker::{self, Placement};

fn temp_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lavatui-model-{name}-{}", std::process::id()));
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

/// A model on a fresh config at `path`. Colour depth is truecolor unless
/// `session` says otherwise, never what the environment (`NO_COLOR`,
/// `COLORTERM`) happens to say; the welcome card starts dismissed (its
/// tests show it themselves).
fn model_with(session: Session, path: PathBuf, cols: u16, rows: u16) -> (Model, Instant) {
    let t0 = Instant::now();
    let session = Session {
        color: session.color.or(Some(ColorChoice::Truecolor)),
        ..session
    };
    let mut model = Model::new(
        &session,
        Store::new(Some(path)),
        Rect::new(0, 0, cols, rows),
        None,
        local(),
        1,
        t0,
    );
    model.welcome = false;
    (model, t0)
}

fn model(name: &str) -> (Model, Instant) {
    model_with(Session::default(), temp_config(name), 80, 24)
}

fn tick(m: &mut Model, at: Instant) {
    let area = m.layout.area;
    m.tick(at, area, local());
}

#[test]
fn esc_does_nothing_and_q_quits() {
    let (mut m, t0) = model("quit");
    m.update(Action::Close, t0);
    assert!(!m.quit);
    m.update(Action::Quit, t0);
    assert!(m.quit);
}

#[test]
fn style_picker_previews_and_reverts() {
    let (mut m, t0) = model("picker-revert");
    let before = m.style;
    m.update(Action::StylePicker, t0);
    assert_eq!(
        m.input_mode(),
        InputMode::Picker {
            opener: Action::StylePicker,
            inline: false
        }
    );
    m.update(Action::Down, t0);
    assert_ne!(m.style, before, "moving the cursor previews live");
    m.update(Action::Close, t0);
    assert_eq!(m.style, before, "esc reverts");
    assert_eq!(m.overlay, Overlay::None);
    assert!(m.save_at.is_none(), "a reverted preview isn't saved");
}

#[test]
fn picker_keep_persists_across_restart() {
    let path = temp_config("picker-keep");
    let (mut m, t0) = model_with(Session::default(), path.clone(), 80, 24);
    m.update(Action::PalettePicker, t0);
    m.update(Action::Jump(2), t0);
    m.update(Action::PalettePicker, t0); // the opening key keeps
    assert_eq!(m.overlay, Overlay::None);
    let name = m.theme.palette().name;
    assert_eq!(name, Palette::all()[2].name);
    assert_eq!(m.settings.theme.palette, name);

    tick(&mut m, t0 + Duration::from_millis(500));
    assert!(!path.exists(), "save is debounced");
    tick(&mut m, t0 + Duration::from_millis(1100));
    assert!(path.exists(), "saved after the debounce");

    let (again, _) = model_with(Session::default(), path, 80, 24);
    assert_eq!(again.theme.palette().name, name);
}

#[test]
fn face_picker_jump_ignores_out_of_range() {
    let (mut m, t0) = model("face-jump");
    m.update(Action::FacePicker, t0);
    m.update(Action::Jump(8), t0);
    assert_eq!(m.face.name(), clock::FACES[0].name());
    m.update(Action::Up, t0);
    assert_eq!(
        m.face.name(),
        clock::FACES[clock::FACES.len() - 1].name(),
        "wraps"
    );
    m.update(Action::Keep, t0);
    assert_eq!(m.settings.clock.face, m.face.name());
}

#[test]
fn cycling_toasts_name_and_position() {
    let (mut m, t0) = model("cycle");
    m.update(Action::NextStyle, t0);
    let n = StyleId::all().len();
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        format!("{}  2/{n}", m.style.style().name())
    );
    m.update(Action::NextFace, t0);
    assert_eq!(m.face.name(), clock::FACES[1].name());
}

#[test]
fn minimal_toggle_is_instant() {
    let (mut m, t0) = model("minimal");
    assert!(m.layout.status.is_some() && m.layout.panel.is_some());
    m.update(Action::ToggleMinimal, t0);
    assert!(m.layout.status.is_none() && m.layout.panel.is_none());
    assert!(m.minimal());
    m.update(Action::ToggleStatusBar, t0);
    assert!(
        m.settings.ui.status_bar,
        "b changes nothing in minimal mode"
    );
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "no status bar in lamp-only mode · m to leave",
        "but says why"
    );
    m.update(Action::ToggleMinimal, t0);
    assert!(m.layout.status.is_some());
}

#[test]
fn global_keys_are_off_while_help_is_open() {
    let (mut m, t0) = model("help");
    m.update(Action::Help, t0);
    assert_eq!(m.input_mode(), InputMode::Help);
    let style = m.style;
    m.update(Action::NextStyle, t0);
    assert_eq!(m.style, style);
    for _ in 0..50 {
        m.update(Action::Down, t0);
    }
    let Overlay::Help { scroll } = m.overlay else {
        panic!("help closed")
    };
    assert_eq!(scroll, crate::ui::help::sheet::max_scroll(m.layout.area));
    m.update(Action::Close, t0);
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn pomodoro_phase_end_toasts_flashes_and_rings() {
    let (mut m, t0) = model("pomodoro");
    m.update(Action::PomodoroToggle, t0);
    assert_eq!(m.pomodoro.status(), Status::Running);
    assert!(Pomodoro.chip(&m).is_some());
    let end = t0 + Duration::from_secs(25 * 60);
    tick(&mut m, end);
    assert_eq!(m.toast.as_ref().unwrap().text, "break · 5:00");
    assert!(m.bell, "bell rings by default");
    assert!(m.flash.is_some());
    tick(&mut m, end + FLASH_TIME);
    assert!(m.flash.is_none());
}

#[test]
fn reset_needs_two_presses_within_two_seconds() {
    let (mut m, t0) = model("reset");
    m.update(Action::PomodoroToggle, t0);
    m.update(Action::PomodoroReset, t0);
    assert_eq!(m.pomodoro.status(), Status::Running);
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "press r again to reset the timer"
    );
    tick(&mut m, t0 + Duration::from_secs(3));
    m.update(Action::PomodoroReset, t0 + Duration::from_secs(3));
    assert_eq!(
        m.pomodoro.status(),
        Status::Running,
        "window expired: re-armed"
    );
    let second = t0 + Duration::from_secs(4);
    m.update(Action::PomodoroReset, second);
    tick(&mut m, second + Duration::from_millis(100));
    assert_eq!(
        m.pomodoro.status(),
        Status::Running,
        "waits to see the second press isn't auto-repeat"
    );
    tick(&mut m, second + Duration::from_millis(160));
    assert_eq!(m.pomodoro.status(), Status::Idle);
    assert_eq!(m.toast.as_ref().unwrap().text, "timer reset");
}

/// lava-ebq.19 (2): terminals without key-release reports send a held key
/// as presses: one, a pause (the repeat delay), then a fast stream. That
/// must never reset, at any common repeat delay or rate.
#[test]
fn holding_r_never_resets() {
    for delay in [150, 250, 500, 660] {
        for rate in [16, 33, 50, 90] {
            let (mut m, t0) = model(&format!("held-r-{delay}-{rate}"));
            m.update(Action::PomodoroToggle, t0);
            let mut at = t0;
            m.update(Action::PomodoroReset, at);
            at += Duration::from_millis(delay);
            for _ in 0..60 {
                m.update(Action::PomodoroReset, at);
                at += Duration::from_millis(rate);
                tick(&mut m, at);
            }
            tick(&mut m, at + Duration::from_secs(1));
            assert_eq!(
                m.pomodoro.status(),
                Status::Running,
                "held r reset it (delay {delay} ms, rate {rate} ms)"
            );
            // A deliberate double press afterwards still works.
            let at = at + Duration::from_secs(3);
            m.update(Action::PomodoroReset, at);
            m.update(Action::PomodoroReset, at + Duration::from_millis(300));
            tick(&mut m, at + Duration::from_millis(500));
            assert_eq!(m.pomodoro.status(), Status::Idle);
        }
    }
}

/// lava-ebq.18: a suspend is seen as the wall clock jumping ahead of
/// `Instant`. A running focus slept through ends once (one bell) and the
/// break starts at wake.
#[test]
fn a_suspend_ends_the_running_phase_once() {
    let (mut m, t0) = model("suspend");
    m.update(Action::PomodoroToggle, t0);
    let at = |secs: u64, wall_secs: u64| {
        let mut l = local();
        l.wall = SystemTime::UNIX_EPOCH + Duration::from_secs(wall_secs);
        (t0 + Duration::from_secs(secs), l)
    };
    // 10 min of focus, then 2 h asleep: Instant moves 1 s, the wall 2 h.
    let (now, l) = at(600, 600);
    m.tick(now, m.layout.area, l);
    assert!(!m.bell);
    let (now, l) = at(601, 601 + 2 * 3600);
    m.tick(now, m.layout.area, l);
    assert!(std::mem::take(&mut m.bell), "the focus ended while asleep");
    assert_eq!(m.toast.as_ref().unwrap().text, "break · 5:00");
    assert_eq!(format_remaining(m.pomodoro.remaining(now)), "5:00");
    for s in 1..=10 {
        let (now, l) = at(601 + s, 601 + 2 * 3600 + s);
        m.tick(now, m.layout.area, l);
        assert!(!m.bell, "exactly one bell");
    }
    // The wall clock going backwards (NTP, a manual change) is ignored.
    let (now, l) = at(620, 0);
    m.tick(now, m.layout.area, l);
    assert_eq!(format_remaining(m.pomodoro.remaining(now)), "4:41");
}

/// lava-ebq.19 (3): growing the window re-clamps the help scroll, so its
/// top lines don't stay scrolled out of view.
#[test]
fn help_scroll_is_clamped_when_the_window_grows() {
    let (mut m, t0) = model_with(Session::default(), temp_config("help-scroll"), 40, 14);
    m.update(Action::Help, t0);
    for _ in 0..50 {
        m.update(Action::Down, t0);
    }
    let Overlay::Help { scroll } = m.overlay else {
        panic!("help is open");
    };
    assert!(scroll > 0, "small help scrolls");
    m.tick(t0, Rect::new(0, 0, 40, 90), local());
    assert_eq!(m.overlay, Overlay::Help { scroll: 0 });
}

#[test]
fn heat_and_speed_clamp_and_reset() {
    let (mut m, t0) = model("heat");
    for _ in 0..9 {
        m.update(Action::HeatUp, t0);
    }
    assert_eq!(m.settings.lamp.heat, 5);
    assert_eq!(m.toast.as_ref().unwrap().text, "heat ▮▮▮▮▮");
    m.update(Action::Faster, t0);
    assert_eq!(m.toast.as_ref().unwrap().text, "speed ×2");
    m.update(Action::ResetHeatSpeed, t0);
    assert_eq!(m.settings.lamp.heat, 3);
    assert_eq!(m.settings.lamp.speed, 1.0);
}

/// lava-ebq.41: the chip names a break, so focus and break differ without
/// colour, running or paused.
#[test]
fn chip_tells_focus_from_break() {
    let (mut m, t0) = model("chip-phase");
    m.update(Action::PomodoroToggle, t0);
    assert_eq!(Pomodoro.chip(&m).unwrap().text, "▸ 25:00");
    m.update(Action::PomodoroSkip, t0);
    let text = Pomodoro.chip(&m).unwrap().text;
    assert!(text.starts_with("▸ break "), "{text}");
    m.update(Action::PomodoroToggle, t0);
    assert!(Pomodoro.chip(&m).unwrap().text.starts_with("‖ break "));
}

#[test]
fn cli_overrides_are_not_written_back() {
    let path = temp_config("override");
    let session = Session {
        minimal: true,
        style: Some("ascii".into()),
        ..Session::default()
    };
    let (mut m, t0) = model_with(session, path.clone(), 80, 24);
    assert!(m.minimal());
    m.update(Action::HeatUp, t0);
    m.save();
    let (again, _) = model_with(Session::default(), path, 80, 24);
    assert!(!again.minimal());
    assert_eq!(again.style.style().name(), "solid");
    assert_eq!(again.settings.lamp.heat, 4);
}

/// The 12th style was renamed glass → chrome (lava-ebq.23); the old name
/// still works in config and on the command line.
#[test]
fn old_glass_style_name_means_chrome() {
    let path = temp_config("glass-alias");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[lamp]\nstyle = \"glass\"\n").unwrap();
    let (m, _) = model_with(Session::default(), path, 80, 24);
    assert_eq!(m.style.style().name(), "chrome");
    let session = Session {
        style: Some("glass".into()),
        ..Session::default()
    };
    let (m, _) = model_with(session, temp_config("glass-alias-cli"), 80, 24);
    assert_eq!(m.style.style().name(), "chrome");
}

#[test]
fn corrupt_config_toasts_and_uses_defaults() {
    let path = temp_config("corrupt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "this is = = not toml").unwrap();
    let (m, _) = model_with(Session::default(), path, 80, 24);
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "config: invalid TOML line 1 · using defaults"
    );
    assert_eq!(m.style, StyleId::default());
}

/// lava-ebq.33: a save writes only what changed in the app, so hand edits
/// made to the file while the lamp runs survive it.
#[test]
fn saving_keeps_hand_edits_made_while_running() {
    let path = temp_config("hand-edit");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[lamp]\nheat = 2\n").unwrap();
    let (mut m, t0) = model_with(Session::default(), path.clone(), 80, 24);
    std::fs::write(&path, "[lamp]\nheat = 5\n\n[pomodoro]\nfocus_min = 50\n").unwrap();
    m.update(Action::Faster, t0);
    m.save();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        saved,
        "[lamp]\nheat = 5\nspeed = 2.0\n\n[pomodoro]\nfocus_min = 50\n"
    );
    assert!(
        m.toast
            .as_ref()
            .is_none_or(|t| !t.text.starts_with("config"))
    );
}

#[test]
fn focus_lowers_the_frame_rate() {
    let (mut m, t0) = model("fps");
    assert_eq!(m.target_fps(), 60);
    assert_eq!(m.idle_until(), None);
    m.update(Action::Focus(false), t0);
    assert_eq!(m.target_fps(), 10);
    m.update(Action::Focus(true), t0);
    assert_eq!(m.target_fps(), 60);
}

/// lava-ebq.3: frozen redraws only when what's shown changes (§7).
#[test]
fn frozen_sleeps_until_the_clock_changes() {
    let (mut m, t0) = model("frozen");
    m.update(Action::Freeze, t0);
    // The "frozen" toast still has to fade: paced as usual.
    assert_eq!(m.idle_until(), None);
    let t1 = t0 + TOAST_TIME;
    tick(&mut m, t1);
    // 14:32:07.000 → the next minute is 53 s away.
    let wake = m.idle_until().expect("idle");
    assert_eq!(wake - t1, Duration::from_secs(53) + WAKE_SLACK);

    // A running pomodoro ticks every second.
    m.update(Action::PomodoroToggle, t1);
    let t2 = t1 + TOAST_TIME + Duration::from_millis(300);
    tick(&mut m, t2);
    let wake = m.idle_until().expect("idle");
    assert!(wake - t2 <= Duration::from_secs(1) + WAKE_SLACK);
    // 1.7 s in: 24:58.3 left (shown 24:59), 24:58 in 0.3 s.
    assert_eq!(wake - t2, Duration::from_millis(300) + WAKE_SLACK);

    // Adaptive quality ignores idle frames.
    m.frame_drawn(500.0, Duration::from_secs(1), t2);
    m.frame_drawn(500.0, Duration::from_secs(3), t2 + Duration::from_secs(3));
    assert!(!m.quality.degraded());
}

/// lava-ebq.3: slow frames drop the grid, then the frame rate (§7).
#[test]
fn slow_frames_degrade_quality() {
    let (mut m, t0) = model("quality");
    let dt = Duration::from_millis(16);
    let mut t = t0;
    for _ in 0..400 {
        t += dt;
        m.frame_drawn(20.0, dt, t);
    }
    assert!(m.quality.reduced_grid());
    assert_eq!(m.target_fps(), 30);
    assert_eq!(
        m.settings.display.fps, 60,
        "the user's setting is untouched"
    );
}

/// lava-ebq.3: clicking a picker item previews it; a double-click keeps.
#[test]
fn clicks_preview_and_double_clicks_keep() {
    let (mut m, t0) = model("click");
    m.update(Action::StylePicker, t0);
    let Overlay::Picker(p) = m.overlay else {
        panic!("picker open")
    };
    let Some(Placement::Sheet { list, .. }) = picker::placement(m.layout.area, &m.layout, &p)
    else {
        panic!("80x24 has a sheet")
    };
    let click = |row: u16| Action::Click {
        col: list.x + 2,
        row: list.y + row,
    };
    m.update(click(2), t0);
    assert_eq!(m.style.index(), 2, "a click previews");
    assert!(matches!(m.overlay, Overlay::Picker(p) if p.cursor == 2));
    // Slow second click: just another preview.
    m.update(click(2), t0 + Duration::from_secs(1));
    assert!(matches!(m.overlay, Overlay::Picker(_)));
    m.update(click(2), t0 + Duration::from_millis(1200));
    assert_eq!(m.overlay, Overlay::None, "a double-click keeps");
    assert_eq!(m.settings.lamp.style, m.style.style().name());
    // Off the sheet: swallowed, nothing changes.
    m.update(Action::StylePicker, t0);
    m.update(Action::Click { col: 0, row: 0 }, t0);
    assert!(matches!(m.overlay, Overlay::Picker(p) if p.cursor == 2));
}

#[test]
fn inline_picker_arrows_click() {
    let (mut m, t0) = model_with(Session::default(), temp_config("inline-click"), 30, 10);
    m.update(Action::StylePicker, t0);
    let Overlay::Picker(p) = m.overlay else {
        panic!("picker open")
    };
    let Some(Placement::Inline { rect, .. }) = picker::placement(m.layout.area, &m.layout, &p)
    else {
        panic!("30x10 is inline")
    };
    m.update(
        Action::Click {
            col: rect.right() - 1,
            row: rect.y,
        },
        t0,
    );
    assert_eq!(m.style.index(), 1, "› is next");
    m.update(
        Action::Click {
            col: rect.x,
            row: rect.y,
        },
        t0,
    );
    assert_eq!(m.style.index(), 0, "‹ is previous");
}

#[test]
fn picker_list_scrolls_only_to_keep_the_cursor_in_view() {
    // 50x16: a bottom sheet with a few rows.
    let (mut m, t0) = model_with(Session::default(), temp_config("scroll"), 50, 16);
    m.update(Action::StylePicker, t0);
    let n = PickerKind::Style.items().len();
    for _ in 0..n - 1 {
        m.update(Action::Down, t0);
    }
    let Overlay::Picker(p) = m.overlay else {
        panic!()
    };
    assert_eq!(p.cursor, n - 1);
    let top = p.top;
    assert!(top > 0, "scrolled to the end");
    // Moving up inside the view doesn't scroll.
    m.update(Action::Up, t0);
    assert!(matches!(m.overlay, Overlay::Picker(p) if p.top == top));
}

#[test]
fn tiny_terminals_use_inline_pickers() {
    let (mut m, t0) = model_with(Session::default(), temp_config("tiny"), 30, 10);
    m.update(Action::StylePicker, t0);
    assert!(matches!(
        m.input_mode(),
        InputMode::Picker { inline: true, .. }
    ));
}

#[test]
fn clicks_outside_the_wax_are_ignored() {
    let (mut m, t0) = model("poke");
    m.update(Action::Poke { col: 0, row: 0 }, t0);
    let view = m.layout.lamp.unwrap();
    m.update(
        Action::Poke {
            col: view.x + view.width / 2,
            row: view.bottom() - 1,
        },
        t0,
    );
    tick(&mut m, t0 + Duration::from_millis(16));
}

#[test]
fn resize_reshapes_the_lamp() {
    let (mut m, t0) = model("resize");
    let before = m.field.aspect();
    m.update(Action::Resize, t0);
    m.tick(
        t0 + Duration::from_millis(16),
        Rect::new(0, 0, 60, 12),
        local(),
    );
    // The sim's view follows the new lamp at once (its walls ease after).
    let lamp = m.layout.lamp.unwrap();
    let aspect = crate::ui::layout::visual_aspect(lamp.width, lamp.height, m.cell_aspect);
    assert_eq!(m.field.aspect(), aspect as f32);
    assert_ne!(m.field.aspect(), before);
}

#[test]
fn flash_level_rises_and_falls_once() {
    let (mut m, t0) = model("flash-level");
    assert_eq!(m.flash_level(), 0.0);
    m.flash = Some(t0);
    m.now = t0 + FLASH_TIME / 2;
    assert!(m.flash_level() > 0.99);
    m.now = t0 + FLASH_TIME / 6;
    assert!((m.flash_level() - 0.5).abs() < 0.01);
    m.now = t0 + FLASH_TIME;
    assert!(m.flash_level() < 0.01);
}

#[test]
fn widget_keys_cycle_places_and_persist() {
    let path = temp_config("dock");
    let (mut m, t0) = model_with(Session::default(), path.clone(), 120, 36);
    let clock = m.layout.panel.as_ref().unwrap().items[0];
    assert_eq!(clock.widget, 0);
    m.update(Action::Place("clock"), t0);
    assert_eq!(m.settings.dock.place(&Clock), Place::Overlay);
    assert_eq!(m.toast.as_ref().unwrap().text, "clock · on the lamp");
    assert_eq!(m.layout.on_lava[0].items[0].widget, 0);
    m.update(Action::Place("pomodoro"), t0);
    assert!(m.layout.panel.is_none(), "nothing left beside the lamp");
    assert_eq!(m.layout.lamp.unwrap().width, 120);
    // Both at the centre: one stack.
    assert_eq!(m.layout.on_lava.len(), 1);
    // `l` moves the one last put there; `L` picks the other.
    m.update(Action::NextAnchor, t0);
    assert_eq!(m.settings.dock.anchor(&Pomodoro), Anchor::Top);
    assert_eq!(m.toast.as_ref().unwrap().text, "timer · top");
    assert_eq!(m.layout.on_lava.len(), 2, "two anchors, two stacks");
    m.update(Action::NextLavaWidget, t0);
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "l moves the clock · now centre"
    );
    m.update(Action::NextAnchor, t0);
    assert_eq!(m.settings.dock.anchor(&Clock), Anchor::Top);
    assert_eq!(m.layout.on_lava.len(), 1, "together again");
    m.update(Action::Place("clock"), t0);
    assert_eq!(m.toast.as_ref().unwrap().text, "clock · off");
    assert!(m.layout.placed(0).is_none());
    m.save_at = Some(t0);
    m.save();

    let (m, _) = model_with(Session::default(), path, 120, 36);
    assert_eq!(m.settings.dock.place(&Clock), Place::Off);
    assert_eq!(m.settings.dock.place(&Pomodoro), Place::Overlay);
    assert_eq!(m.settings.dock.anchor(&Pomodoro), Anchor::Top);
    assert_eq!(m.settings.dock.anchor(&Clock), Anchor::Top);
}

#[test]
fn widget_toasts_say_when_there_is_no_room() {
    let (mut m, t0) = model_with(Session::default(), temp_config("dock-room"), 24, 9);
    m.update(Action::Place("clock"), t0);
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "clock · on the lamp · enlarge to see"
    );
    assert!(m.layout.chipped(0), "the chip stands in");
    m.update(Action::ToggleMinimal, t0);
    m.update(Action::Place("clock"), t0);
    m.update(Action::Place("clock"), t0);
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "clock · beside the lamp · hidden in lamp-only mode"
    );
    m.update(Action::NextAnchor, t0);
    assert_eq!(m.toast.as_ref().unwrap().text, actions::NOTHING_ON_LAMP);
}

// --- music (lava-75z.2) --------------------------------------------------

mod music {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::dock::Music;
    use crate::media::art::{Art, ArtLoader};
    use crate::media::{
        Capabilities, Command, FakeSource, MediaSource, Snapshot, Track, Unavailable,
    };
    use crate::theme::Rgb;
    use crate::ui::keymap::PlayerKey as P;

    const S: Duration = Duration::from_secs(1);
    const COVER: &str = "https://i.example/cover";

    /// A source that counts how many of it are alive.
    struct Counted(FakeSource, Arc<AtomicUsize>);

    impl MediaSource for Counted {
        fn snapshot(&self) -> Snapshot {
            self.0.snapshot()
        }
        fn send(&self, command: Command) {
            self.0.send(command);
        }
        fn capabilities(&self) -> Capabilities {
            self.0.capabilities()
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.1.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn fake(now: Instant) -> FakeSource {
        let track = Track {
            id: "fake:1".into(),
            uri: None,
            name: "Slow Rise".into(),
            artist: "The Paraffins".into(),
            album: "Heat Rises".into(),
            duration: S * 214,
            artwork_url: COVER.into(),
        };
        FakeSource::new(
            Snapshot {
                track: Some(std::sync::Arc::new(track)),
                position: S * 42,
                volume: 70,
                ..Snapshot::new(crate::media::Status::Playing, now)
            },
            Vec::new(),
        )
    }

    /// A model with `source` as its player (alive count in the `Arc`).
    fn with(m: &mut Model, source: &FakeSource) -> Arc<AtomicUsize> {
        let alive = Arc::new(AtomicUsize::new(0));
        let (source, count) = (source.clone(), Arc::clone(&alive));
        m.music.connect_with(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Box::new(Counted(source.clone(), Arc::clone(&count)))
            },
            || ArtLoader::preloaded(COVER, Art::solid(Rgb(200, 120, 40))),
        );
        alive
    }

    fn keys(m: &mut Model, t: Instant, keys: &[P]) {
        for &k in keys {
            m.update(Action::Player(k), t);
        }
    }

    #[test]
    fn off_by_default_and_never_connects() {
        let (mut m, t0) = model("music-off");
        let alive = with(&mut m, &fake(t0));
        tick(&mut m, t0);
        assert_eq!(m.settings.dock.place(&Music), Place::Off);
        assert_eq!(alive.load(Ordering::SeqCst), 0);
        assert!(m.music.snapshot.is_none());
    }

    #[test]
    fn connects_while_placed_and_lets_go_when_off() {
        // This test requires a cover, regardless of the test runner's
        // TERM / NO_COLOR environment.
        let session = Session {
            color: Some(crate::config::ColorChoice::Truecolor),
            ..Session::default()
        };
        let (mut m, t0) = model_with(session, temp_config("music-life"), 120, 36);
        let alive = with(&mut m, &fake(t0));
        m.update(Action::Place("music"), t0);
        assert_eq!(m.toast.as_ref().unwrap().text, "music · beside the lamp");
        tick(&mut m, t0);
        assert_eq!(alive.load(Ordering::SeqCst), 1);
        let placed = m.layout.placed(2).expect("music in the panel");
        assert_eq!(
            placed.form.variant & 0xff00,
            0x200,
            "the cover beside the card"
        );
        m.update(Action::Place("music"), t0); // on the lava: same source
        tick(&mut m, t0);
        assert_eq!(alive.load(Ordering::SeqCst), 1);
        assert!(!m.layout.on_lava.is_empty());
        m.update(Action::Place("music"), t0); // off: dropped, polling stops
        tick(&mut m, t0);
        assert_eq!(alive.load(Ordering::SeqCst), 0);
        assert!(m.music.snapshot.is_none());
    }

    #[test]
    fn player_keys_send_commands() {
        let (mut m, t0) = model("music-keys");
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("music"), t0);
        tick(&mut m, t0);
        m.update(Action::PlayerKeys, t0);
        assert_eq!(m.input_mode(), InputMode::Player);
        keys(
            &mut m,
            t0,
            &[
                P::PlayPause,
                P::PlayPause,
                P::SeekForward,
                P::SeekBack,
                P::VolumeUp,
                P::VolumeDown,
                P::VolumeDown,
                P::Next,
                P::Previous,
            ],
        );
        let sent = source.sent();
        assert_eq!(sent[..2], [Command::PlayPause, Command::PlayPause]);
        let Command::Seek(fwd) = sent[2] else {
            panic!("{sent:?}")
        };
        let Command::Seek(back) = sent[3] else {
            panic!("{sent:?}")
        };
        assert!(fwd >= S * 52 && fwd < S * 53, "{fwd:?}");
        assert!(back >= S * 42 && back < S * 43, "{back:?}");
        assert_eq!(
            sent[4..],
            [
                Command::SetVolume(75),
                Command::SetVolume(70),
                Command::SetVolume(65),
                Command::Next,
                Command::Previous,
            ]
        );
        assert_eq!(m.toast.as_ref().unwrap().text, "volume 65");
        // Seen at once in the widget's snapshot (optimistic).
        assert_eq!(m.music.snapshot.as_ref().unwrap().volume, 65);
    }

    #[test]
    fn shuffle_and_repeat_only_where_the_player_has_them() {
        let (mut m, t0) = model("music-shuffle");
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("music"), t0);
        tick(&mut m, t0);
        m.update(Action::PlayerKeys, t0);
        keys(&mut m, t0, &[P::Shuffle, P::Repeat]);
        assert_eq!(
            source.sent(),
            [Command::SetShuffle(true), Command::SetRepeat(true)]
        );
        // Spotify's AppleScript can't (lava-75z.9): nothing is sent.
        source.set_capabilities(Capabilities::NONE);
        keys(&mut m, t0, &[P::Shuffle, P::Repeat]);
        assert_eq!(source.sent().len(), 2);
        assert!(
            m.toast.as_ref().unwrap().text.contains("can't shuffle"),
            "{:?}",
            m.toast
        );
    }

    #[test]
    fn before_the_first_answer_it_still_has_a_form() {
        // No forms would take the whole panel (clock, pomodoro) down.
        let (mut m, t0) = model("music-early");
        with(&mut m, &fake(t0));
        m.settings.dock.set(&Music, Place::Side);
        m.music.snapshot = None;
        assert!(!Music.forms(&m, Place::Side).is_empty());
        m.relayout(m.layout.area);
        assert!(m.layout.panel.is_some());
    }

    #[test]
    fn the_player_keys_are_a_mode() {
        let (mut m, t0) = model("music-mode");
        m.update(Action::PlayerKeys, t0);
        assert_eq!(m.input_mode(), InputMode::Normal, "nothing to control");
        assert_eq!(
            m.toast.as_ref().unwrap().text,
            "music is off · a to show it"
        );

        with(&mut m, &fake(t0));
        m.update(Action::Place("music"), t0);
        tick(&mut m, t0);
        m.update(Action::PlayerKeys, t0);
        assert_eq!(m.input_mode(), InputMode::Player);
        let style = m.style;
        m.update(Action::NextStyle, t0);
        assert_eq!(m.style, style, "global keys wait");
        m.update(Action::Close, t0);
        assert_eq!(m.input_mode(), InputMode::Normal);
        m.update(Action::PlayerKeys, t0);
        m.update(Action::Help, t0);
        assert_eq!(m.input_mode(), InputMode::Help);
        assert!(!m.music.keys, "help ends them");
        m.update(Action::Close, t0);
        m.update(Action::PlayerKeys, t0);
        m.update(Action::Place("music"), t0); // passes? no: keys are a mode
        assert_eq!(m.settings.dock.place(&Music), Place::Side);
        m.update(Action::Quit, t0);
        assert!(m.quit, "quit always gets through");
    }

    #[test]
    fn without_a_player_it_says_why_calmly() {
        let (mut m, t0) = model_with(Session::default(), temp_config("music-states"), 120, 36);
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("music"), t0);
        for (reason, says) in [
            (Unavailable::NotRunning, "Open Spotify to show music"),
            (Unavailable::NotInstalled, "Spotify is not installed"),
            (Unavailable::PermissionDenied, "Spotify"),
            (Unavailable::Unsupported, "No media player"),
        ] {
            source.set(Snapshot {
                player: Some("Spotify".into()),
                ..Snapshot::new(crate::media::Status::Unavailable(reason), t0)
            });
            tick(&mut m, t0);
            let forms = Music.forms(&m, Place::Side);
            assert_eq!(forms.len(), 1, "one calm message");
            assert!(forms[0].size.width <= 20 && forms[0].size.height <= 6);
            // Its chip: the next step, or where to read it.
            let chip = Music.chip(&m).map(|c| c.text);
            let want = match says {
                "Open Spotify to show music" => Some("♪ open Spotify"),
                "No media player" => None,
                _ => Some("♪ see Shift+A"),
            };
            assert_eq!(chip.as_deref(), want);
            m.update(Action::PlayerKeys, t0);
            m.update(Action::Player(P::PlayPause), t0);
            assert!(
                m.toast.as_ref().unwrap().text.contains(says),
                "{:?}",
                m.toast
            );
            m.update(Action::Close, t0);
        }
        assert!(source.sent().is_empty(), "nothing sent to no player");
    }

    #[test]
    fn the_chip_names_the_track_and_outranks_the_clock_while_playing() {
        let (mut m, t0) = model("music-chip");
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("music"), t0);
        m.update(Action::ToggleMinimal, t0);
        tick(&mut m, t0);
        let chip = Music.chip(&m).unwrap();
        assert_eq!(chip.text, "▶ Slow Rise – The Paraffins");
        let row = m.layout.chips.as_ref().unwrap();
        let shown: Vec<usize> = row.items.iter().map(|c| c.widget).collect();
        assert_eq!(shown, [0, 2], "the clock and music, in registry order");
        assert_eq!(Music.rank(&m), 2);
        m.update(Action::PlayerKeys, t0);
        m.update(Action::Player(P::PlayPause), t0);
        m.update(Action::Close, t0);
        assert_eq!(Music.chip(&m).unwrap().text, "‖ Slow Rise – The Paraffins");
        assert_eq!(Music.rank(&m), 1, "paused: below the clock again");
    }

    #[test]
    fn volume_is_left_alone_where_the_player_has_none() {
        let (mut m, t0) = model("music-no-volume");
        let source = fake(t0);
        with(&mut m, &source);
        m.settings.art.inline = false;
        m.update(Action::Place("music"), t0);
        tick(&mut m, t0);
        let card = |m: &Model| {
            let form = Music.forms(m, Place::Side)[0];
            assert_eq!(form.size.height, 6, "the card");
            let area = Rect::new(0, 0, 30, 6);
            let mut buf = ratatui::buffer::Buffer::empty(area);
            let look = crate::dock::Look {
                backdrop: crate::dock::Backdrop::Panel,
                align: ratatui::layout::Alignment::Left,
            };
            Music.draw(m, form, area, look, &mut buf);
            (0..6)
                .map(|y| (0..30).map(|x| buf[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(card(&m).contains("vol 70"), "{}", card(&m));
        // SMTC (Windows) has no volume: no readout, the keys say so.
        source.set_capabilities(Capabilities {
            volume: false,
            ..Capabilities::ALL
        });
        assert!(!card(&m).contains("vol"), "{}", card(&m));
        m.update(Action::PlayerKeys, t0);
        keys(&mut m, t0, &[P::VolumeUp, P::VolumeDown]);
        assert!(source.sent().is_empty(), "{:?}", source.sent());
        assert_eq!(
            m.toast.as_ref().unwrap().text,
            "The player has no volume control here"
        );
    }

    // --- music troubleshooting (lava-1xk.6, .12, .15) ----------------------

    /// The player's problem, as `source` reports it.
    fn unavailable(source: &FakeSource, reason: Unavailable, t0: Instant) {
        source.set(Snapshot {
            player: Some("Spotify".into()),
            ..Snapshot::new(crate::media::Status::Unavailable(reason), t0)
        });
    }

    #[test]
    fn the_music_controls_always_say_how_to_leave() {
        for (cols, rows, minimal) in [
            (80, 24, false),
            (80, 24, true),
            (20, 8, false),
            (12, 5, true),
        ] {
            let (mut m, t0) = model_with(Session::default(), temp_config("guide"), cols, rows);
            let source = fake(t0);
            with(&mut m, &source);
            m.update(Action::Place("music"), t0);
            if minimal {
                m.update(Action::ToggleMinimal, t0);
            }
            m.update(Action::PlayerKeys, t0);
            // Long after the toast has gone.
            tick(&mut m, t0 + S * 5);
            assert!(m.toast.is_none());
            let (_, text) = crate::ui::cards::guide(&m.layout, &m)
                .unwrap_or_else(|| panic!("{cols}x{rows}: no guide"));
            assert!(text.contains("Esc back"), "{cols}x{rows}: {text:?}");
            m.update(Action::Close, t0);
            assert!(crate::ui::cards::guide(&m.layout, &m).is_none());
        }
    }

    #[test]
    fn a_player_problem_keeps_a_chip_and_a_note_when_the_widget_cant_fit() {
        let (mut m, t0) = model_with(Session::default(), temp_config("note"), 80, 24);
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("music"), t0);
        m.update(Action::ToggleMinimal, t0); // side widgets: chips only
        unavailable(&source, Unavailable::PermissionDenied, t0);
        tick(&mut m, t0);
        let (music, _) = crate::dock::by_name("music").unwrap();
        let chips = m.layout.chips.as_ref().unwrap();
        assert!(chips.items.iter().any(|c| c.widget == music), "{chips:?}");
        assert_eq!(Music.chip(&m).unwrap().text, "♪ see Shift+A");
        assert!(crate::ui::cards::music_note(&m.layout, &m).is_none());
        m.update(Action::PlayerKeys, t0);
        let (_, lines) = crate::ui::cards::music_note(&m.layout, &m).expect("the note");
        let said = lines.join(" ");
        let want = Unavailable::PermissionDenied.message("Spotify");
        assert_eq!(said, want, "the whole sentence, wrapped");
        // At 20x8 the not-running chip still outlasts the clock's.
        let (mut m, t0) = model_with(Session::default(), temp_config("note-tiny"), 20, 8);
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("music"), t0);
        unavailable(&source, Unavailable::NotRunning, t0);
        tick(&mut m, t0);
        let chips = m.layout.chips.as_ref().unwrap();
        assert_eq!(chips.items.len(), 1);
        assert_eq!(chips.items[0].widget, music);
        assert_eq!(Music.chip(&m).unwrap().text, "♪ open Spotify");
    }

    #[test]
    fn the_cover_says_the_players_problem_too() {
        let (mut m, t0) = model("cover-problem");
        let source = fake(t0);
        with(&mut m, &source);
        m.update(Action::Place("cover"), t0);
        for (reason, says) in [
            (Unavailable::NotRunning, "Open Spotify to show album art"),
            (Unavailable::NotInstalled, "Spotify is not installed"),
            (Unavailable::PermissionDenied, "Spotify"),
        ] {
            unavailable(&source, reason.clone(), t0);
            tick(&mut m, t0);
            let forms = Cover.forms(&m, Place::Side);
            assert_eq!(forms.len(), 1, "{reason:?}: one message");
            let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 30, 10));
            let rect = Rect::new(0, 0, forms[0].size.width, forms[0].size.height);
            let look = crate::dock::Look {
                backdrop: crate::dock::Backdrop::Panel,
                align: ratatui::layout::Alignment::Left,
            };
            Cover.draw(&m, forms[0], rect, look, &mut buf);
            let text: String = (0..10)
                .map(|y| {
                    (0..30)
                        .map(|x| buf[(x, y)].symbol().to_owned())
                        .collect::<String>()
                        .trim()
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains(says), "{reason:?}: {text:?}");
            assert!(!text.contains("nothing playing"), "{reason:?}");
        }
    }

    // --- the cover widget (lava-75z.13) ----------------------------------

    use crate::dock::Cover;
    use crate::dock::cover::{Caps, CoverSize, Detail};

    /// The cover's index in the registry.
    const COVER_W: usize = 4;

    fn with_hires(m: &mut Model, source: &FakeSource) {
        let source = source.clone();
        m.music.connect_with(
            move || Box::new(source.clone()),
            || ArtLoader::preloaded(COVER, Art::solid(Rgb(200, 120, 40)).with_hires("QUJD")),
        );
    }

    #[test]
    fn the_cover_is_a_widget_of_its_own_sized_by_the_setting() {
        let (mut m, t0) = model_with(Session::default(), temp_config("cover-place"), 120, 36);
        with(&mut m, &fake(t0));
        assert_eq!(m.settings.dock.place(&Cover), Place::Off, "off by default");
        m.update(Action::Place("music"), t0);
        tick(&mut m, t0);
        let inline = |m: &Model| Music.forms(m, Place::Side)[0].size == (32, 6).into();
        assert!(inline(&m), "the card keeps a small cover of its own");
        m.update(Action::Place("cover"), t0);
        assert_eq!(m.toast.as_ref().unwrap().text, "cover · beside the lamp");
        tick(&mut m, t0);
        assert!(!inline(&m), "one cover at a time");
        let p = m.layout.placed(COVER_W).expect("in the panel");
        assert_eq!((p.form.size.width, p.form.size.height), (24, 12));
        m.settings.art.size = CoverSize::Small;
        tick(&mut m, t0);
        let p = m.layout.placed(COVER_W).unwrap();
        assert_eq!((p.form.size.width, p.form.size.height), (16, 8));
        assert_eq!(Cover.rank(&m), 2, "the music widget's rank");
        assert!(Cover.chip(&m).is_none());
        // On the lava, top right by default.
        m.update(Action::Place("cover"), t0);
        tick(&mut m, t0);
        let lamp = m.layout.lamp.unwrap();
        let p = m.layout.placed(COVER_W).expect("on the lava");
        assert!(p.rect.x > lamp.x + lamp.width / 2 && p.rect.y < lamp.y + lamp.height / 2);
        // A click on it is play / pause.
        let source = fake(t0);
        with(&mut m, &source);
        tick(&mut m, t0);
        let p = m.layout.placed(COVER_W).unwrap();
        m.update(
            Action::Press {
                col: p.rect.x + 2,
                row: p.rect.y + 1,
            },
            t0,
        );
        assert_eq!(source.sent(), [Command::PlayPause]);
    }

    #[test]
    fn without_colours_for_it_the_cover_is_one_calm_line() {
        let session = Session {
            color: Some(crate::config::ColorChoice::Ansi16),
            ..Session::default()
        };
        let (mut m, t0) = model_with(session, temp_config("cover-16"), 120, 36);
        with_hires(&mut m, &fake(t0));
        m.update(Action::Place("cover"), t0);
        tick(&mut m, t0);
        let forms = Cover.forms(&m, Place::Side);
        assert_eq!(forms.len(), 1);
        assert!(forms[0].size.width <= 20, "{forms:?}");
        assert_eq!(Cover.rank(&m), 0, "a message gives way");
        assert!(m.layout.panel.is_some(), "and takes nothing down");
        // A terminal with real pixels doesn't need the palette's colours.
        m.caps = Caps {
            pixels: Some(crate::graphics::Protocol::Kitty),
            sextants: true,
        };
        tick(&mut m, t0);
        assert_eq!(Cover.forms(&m, Place::Side)[0].size.width, 24);
    }

    /// lava-1xk.20: inside Ghostex (zmx) no cover setting ever writes a
    /// sextant (U+1FB00–U+1FB3B) or a kitty placeholder (U+10EEEE): the
    /// frame's real bytes, through the crossterm backend. Natively the
    /// same cover does use sextants, so the check means something.
    #[test]
    fn inside_ghostex_covers_use_only_glyphs_it_draws() {
        use crate::dock::cover::sextants;
        const GHOSTEX: &[(&str, &str)] = &[
            ("TERM", "xterm-ghostty"),
            ("TERM_PROGRAM", "ghostty"),
            ("ZMX_SESSION", "s1"),
            ("GHOSTEX_SESSION_ID", "test-session"),
        ];
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_owned())
        };
        let inside = Caps {
            pixels: crate::graphics::detect(env(GHOSTEX)),
            sextants: sextants(env(GHOSTEX)),
        };
        assert_eq!(inside, Caps::default(), "no pixels, no sextants");
        let native = Caps {
            pixels: None,
            sextants: sextants(env(&GHOSTEX[..2])),
        };
        assert!(native.sextants);

        let bytes = |caps: Caps, detail: Detail| -> String {
            let (mut m, t0) = model_with(Session::default(), temp_config("ghostex"), 120, 36);
            m.caps = caps;
            // A busy picture: every cell splits two ways.
            let art = Art::from_fn(|x, y| {
                let on = (x / 3 + y / 5) % 2 == 0;
                if on {
                    Rgb(230, 90, 30)
                } else {
                    Rgb(20, 30, 90)
                }
            });
            m.music.connect_with(
                {
                    let source = fake(t0);
                    move || Box::new(source.clone())
                },
                move || ArtLoader::preloaded(COVER, art.clone()),
            );
            m.update(Action::Place("cover"), t0);
            m.settings.art.detail = detail;
            tick(&mut m, t0);
            tick(&mut m, t0);
            let backend = ratatui::backend::CrosstermBackend::new(Vec::<u8>::new());
            // Fixed: a fullscreen terminal would ask the real one its size.
            let options = ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(m.layout.area),
            };
            let mut terminal = ratatui::Terminal::with_options(backend, options).unwrap();
            let mut lamp = crate::render::LampState::default();
            terminal
                .draw(|f| crate::ui::draw(f, &m, &mut lamp))
                .unwrap();
            m.kitty.write(terminal.backend_mut().writer_mut()).unwrap();
            String::from_utf8_lossy(terminal.backend().writer()).into_owned()
        };
        let forbidden = |c: char| ('\u{1FB00}'..='\u{1FB3B}').contains(&c) || c == '\u{10EEEE}';
        for detail in Detail::ALL {
            let out = bytes(inside, detail);
            assert!(
                out.contains('▀') || out.contains('▌') || out.contains('▖'),
                "{detail:?}: a cover"
            );
            let bad: Vec<char> = out.chars().filter(|&c| forbidden(c)).collect();
            assert!(bad.is_empty(), "{detail:?}: {} forbidden glyphs", bad.len());
        }
        assert!(bytes(native, Detail::Sharp).chars().any(forbidden));
    }

    #[test]
    fn pixels_are_sent_once_drawn_as_placeholders_and_deleted_when_off() {
        let (mut m, t0) = model_with(Session::default(), temp_config("cover-kitty"), 120, 36);
        m.caps = Caps {
            pixels: Some(crate::graphics::Protocol::Kitty),
            sextants: true,
        };
        with_hires(&mut m, &fake(t0));
        m.update(Action::Place("cover"), t0);
        tick(&mut m, t0);
        let p = *m.layout.placed(COVER_W).unwrap();
        let draw = |m: &Model| {
            let mut buf = ratatui::buffer::Buffer::empty(m.layout.area);
            let look = crate::dock::Look {
                backdrop: crate::dock::Backdrop::Panel,
                align: ratatui::layout::Alignment::Left,
            };
            Cover.draw(m, p.form, p.rect, look, &mut buf);
            buf[(p.rect.x, p.rect.y)].clone()
        };
        // On its way: sextants meanwhile.
        assert!(m.kitty.busy());
        assert!(!crate::graphics::is_placeholder(draw(&m).symbol()));
        let mut out = Vec::new();
        m.kitty.write(&mut out).unwrap();
        let sent = String::from_utf8(out).unwrap();
        assert!(sent.starts_with("\x1b_Ga=T,U=1,f=100,t=d,i="), "{sent:?}");
        assert!(sent.contains(",c=24,r=12,q=2,m=0;QUJD\x1b\\"), "{sent:?}");
        // There: placeholders in the image id's colour.
        tick(&mut m, t0);
        let cell = draw(&m);
        assert!(crate::graphics::is_placeholder(cell.symbol()));
        assert!(matches!(cell.fg, ratatui::style::Color::Rgb(..)));
        // Nothing more while it stays.
        for _ in 0..3 {
            tick(&mut m, t0);
            let mut out = Vec::new();
            m.kitty.write(&mut out).unwrap();
            assert!(out.is_empty());
        }
        // Resized (fill size): sent again at the new size.
        m.settings.art.size = CoverSize::Large;
        tick(&mut m, t0);
        let mut out = Vec::new();
        m.kitty.write(&mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains(",c=34,r=17,"));
        tick(&mut m, t0);
        let mut out = Vec::new();
        m.kitty.write(&mut out).unwrap();
        assert!(
            String::from_utf8(out).unwrap().contains("a=d,d=I"),
            "the old one"
        );
        // Off: deleted.
        m.update(Action::Place("cover"), t0);
        m.update(Action::Place("cover"), t0);
        assert_eq!(m.settings.dock.place(&Cover), Place::Off);
        tick(&mut m, t0);
        let mut out = Vec::new();
        m.kitty.write(&mut out).unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("\x1b_Ga=d,d=I,i=")
        );
        assert!(!m.kitty.busy());
    }

    /// lava-bq0: in a terminal with pictures, pixelated and chunky are
    /// pictures too, their own (pixel art made with the sharp copy), sent
    /// once each like any other.
    #[test]
    fn pixel_art_is_sent_as_its_own_picture() {
        let (mut m, t0) = model_with(Session::default(), temp_config("cover-pixel-art"), 120, 36);
        m.caps = Caps {
            pixels: Some(crate::graphics::Protocol::Kitty),
            sextants: true,
        };
        with_hires(&mut m, &fake(t0));
        m.update(Action::Place("cover"), t0);
        let sent = |m: &mut Model| {
            tick(m, t0);
            let mut out = Vec::new();
            m.kitty.write(&mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        assert!(sent(&mut m).contains(";QUJD\x1b\\"), "the sharp picture");
        for (detail, png) in [(Detail::Pixelated, "QUJD16"), (Detail::Chunky, "QUJD8")] {
            m.settings.art.detail = detail;
            let out = sent(&mut m);
            assert!(
                out.contains(&format!(";{png}\x1b\\")),
                "{detail:?}: {out:?}"
            );
            assert!(sent(&mut m).contains("a=d,d=I"), "the old one goes");
            assert!(sent(&mut m).is_empty(), "and nothing more");
        }
        // While it's on its way: the same grain in text cells.
        m.settings.art.detail = Detail::Pixelated;
        tick(&mut m, t0);
        let p = *m.layout.placed(COVER_W).unwrap();
        let mut buf = ratatui::buffer::Buffer::empty(m.layout.area);
        let look = crate::dock::Look {
            backdrop: crate::dock::Backdrop::Panel,
            align: ratatui::layout::Alignment::Left,
        };
        Cover.draw(&m, p.form, p.rect, look, &mut buf);
        let glyph = buf[(p.rect.x, p.rect.y)].symbol().to_owned();
        assert!(glyph == "█" || glyph == "▀", "{glyph:?}");
    }

    #[test]
    fn iterm_pictures_are_placed_over_skipped_cells_and_repainted_when_off() {
        use crate::graphics::inline::SENTINEL;
        use ratatui::buffer::{Buffer, CellDiffOption};
        let (mut m, t0) = model_with(Session::default(), temp_config("cover-iterm"), 120, 36);
        m.caps = Caps {
            pixels: Some(crate::graphics::Protocol::Iterm),
            sextants: true,
        };
        with_hires(&mut m, &fake(t0));
        m.update(Action::Place("cover"), t0);
        // One frame as the loop draws it: the cover's cells, settle, the
        // bytes after.
        let frame = |m: &mut Model| {
            tick(m, t0);
            let p = *m.layout.placed(COVER_W).unwrap();
            let mut buf = Buffer::empty(m.layout.area);
            let look = crate::dock::Look {
                backdrop: crate::dock::Backdrop::Panel,
                align: ratatui::layout::Alignment::Left,
            };
            Cover.draw(m, p.form, p.rect, look, &mut buf);
            m.inline.settle(&mut buf);
            let mut out = Vec::new();
            m.inline.write(&mut out).unwrap();
            m.kitty.write(&mut out).unwrap();
            (p.rect, buf, String::from_utf8(out).unwrap())
        };
        let (r, buf, out) = frame(&mut m);
        assert!(
            out.starts_with(&format!(
                "\x1b7\x1b[{};{}H\x1b]1337;File=inline=1;size=3;width=24;height=12;",
                r.y + 1,
                r.x + 1
            )),
            "{out:?}"
        );
        assert!(out.ends_with(":QUJD\x07\x1b8"), "{out:?}");
        assert!(!out.contains("\x1b_G"), "no kitty bytes");
        assert!(r.positions().all(|p| buf[p].symbol() == " "));
        // Left alone after.
        let (_, buf, out) = frame(&mut m);
        assert!(out.is_empty());
        assert!(
            r.positions()
                .all(|p| buf[p].diff_option == CellDiffOption::Skip)
        );
        assert!(r.positions().all(|p| buf[p].symbol() == SENTINEL));
        // Off: repainted where it was.
        m.update(Action::Place("cover"), t0);
        m.update(Action::Place("cover"), t0);
        tick(&mut m, t0);
        let mut buf = Buffer::empty(m.layout.area);
        m.inline.settle(&mut buf);
        assert!(
            r.positions()
                .all(|p| buf[p].diff_option == CellDiffOption::AlwaysUpdate)
        );
        assert!(!m.inline.busy());
    }

    #[test]
    fn pixels_wait_for_the_terminal_to_confirm_them() {
        use crate::dock::cover::Drawn;
        use crate::graphics::Protocol;
        use crate::graphics::probe::{Probe, WAIT};
        let setup = |name: &str| {
            let (mut m, t0) = model_with(Session::default(), temp_config(name), 120, 36);
            m.caps.sextants = true;
            m.probe = Some(Probe::new(Protocol::Kitty, t0));
            with_hires(&mut m, &fake(t0));
            m.update(Action::Place("cover"), t0);
            tick(&mut m, t0);
            m.toast = None;
            (m, t0)
        };
        // Native Ghostty: OK. Text cells until then, pixels after.
        let (mut m, t0) = setup("probe-ok");
        assert_eq!(
            m.pictures(),
            Drawn::Text(crate::dock::picture::TextMode::Sextant)
        );
        assert!(
            m.idle_until()
                .is_none_or(|at| at <= t0 + WAIT + Duration::from_millis(50))
        );
        m.terminal_replies(&["]11;rgb:0/0/0".into(), "_Gi=31;OK".into()]);
        assert!(m.probe.is_none());
        assert_eq!(
            m.pictures(),
            Drawn::Pixels(Protocol::Kitty, crate::dock::cover::Grain::Sharp)
        );
        assert!(m.toast.is_none());
        // The sharp copy is asked for now (the test loader can't fetch it:
        // a fresh one has it).
        with_hires(&mut m, &fake(t0));
        tick(&mut m, t0);
        assert!(m.kitty.busy(), "and the cover goes out");
        // Ghostex-like: the fence comes back first. Text, and a toast.
        let (mut m, t0) = setup("probe-no");
        m.terminal_replies(&["]10;rgb:ffff/ffff/ffff".into()]);
        assert!(m.probe.is_none());
        assert_eq!(m.caps.pixels, None);
        assert_eq!(
            m.pictures(),
            Drawn::Text(crate::dock::picture::TextMode::Sextant)
        );
        assert_eq!(
            m.toast.as_ref().unwrap().text,
            "no photos in this terminal · covers drawn in text"
        );
        tick(&mut m, t0);
        assert!(!m.kitty.busy(), "nothing sent");
        // No answer at all: settled at the deadline, the same way.
        let (mut m, t0) = setup("probe-silent");
        tick(&mut m, t0 + WAIT / 2);
        assert!(m.probe.is_some());
        tick(&mut m, t0 + WAIT);
        assert!(m.probe.is_none());
        assert_eq!(m.caps.pixels, None);
        assert!(m.toast.is_some());
        // A late OK changes nothing.
        m.terminal_replies(&["_Gi=31;OK".into()]);
        assert_eq!(m.caps.pixels, None);
    }

    #[test]
    fn the_detail_key_cycles_and_says_what_it_comes_to() {
        let (mut m, t0) = model("cover-detail");
        let mut seen = Vec::new();
        for _ in 0..Detail::ALL.len() {
            m.update(Action::CoverDetail, t0);
            seen.push(m.toast.as_ref().unwrap().text.clone());
        }
        assert_eq!(
            seen,
            [
                "cover quality · sharp",
                "cover quality · pixelated",
                "cover quality · chunky",
                "cover quality · auto · sharp",
            ]
        );
        assert_eq!(m.settings.art.detail, Detail::Auto);
    }

    #[test]
    fn a_frozen_lamp_still_looks_at_the_player_each_second() {
        let (mut m, t0) = model("music-frozen");
        with(&mut m, &fake(t0));
        m.update(Action::Freeze, t0);
        m.toast = None;
        tick(&mut m, t0);
        let off = m.idle_until().unwrap();
        m.update(Action::Place("music"), t0);
        m.toast = None;
        tick(&mut m, t0);
        let on = m.idle_until().unwrap();
        assert!(on <= t0 + S + Duration::from_millis(10), "{:?}", on - t0);
        assert!(on <= off);
    }
}

#[test]
fn background_save_flushes_cli_overrides_on_quit() {
    let path = temp_config("background-quit");
    let session = Session {
        minimal: true,
        style: Some("braille".into()),
        ..Session::default()
    };
    let (mut model, t0) = model_with(session, path.clone(), 80, 24);
    model.background_saves().unwrap();
    model.update(Action::HeatUp, t0);
    // Quit before the debounce expires, without another tick.
    model.finish_saves();
    let (loaded, _) = model_with(Session::default(), path, 80, 24);
    assert!(!loaded.minimal());
    assert_eq!(loaded.settings.lamp.heat, 4);
    assert_eq!(loaded.style.style().name(), "solid");
}

// --- lyrics (lava-75z.4) -------------------------------------------------

mod lyrics {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::dock::{Lyrics, Music};
    use crate::lyrics::client::tests::{Mock, ok, status};
    use crate::lyrics::client::{Lrclib, Reply};
    use crate::lyrics::{Lyrics as Words, LyricsService};
    use crate::media::{FakeSource, Snapshot, Status as Player, Track, Unavailable};

    const S: Duration = Duration::from_secs(1);
    const LRC: &str =
        "[00:05.00]first line\\n[00:10.00]second line\\n[00:15.00]\\n[00:20.00]third line";

    fn record(synced: &str) -> Result<Reply, String> {
        ok(&format!(
            r#"{{"trackName":"Slow Rise","artistName":"The Paraffins","duration":214.0,
                "instrumental":false,"plainLyrics":"x","syncedLyrics":"{synced}"}}"#
        ))
    }

    fn track(id: &str, name: &str) -> Track {
        Track {
            id: id.into(),
            uri: crate::media::spotify_track_uri(id),
            name: name.into(),
            artist: "The Paraffins".into(),
            album: "Heat Rises".into(),
            duration: S * 214,
            artwork_url: String::new(),
        }
    }

    fn playing(t: Track, position: Duration, at: Instant) -> Snapshot {
        Snapshot {
            player: Some("Spotify".into()),
            track: Some(Arc::new(t)),
            position,
            volume: 70,
            ..Snapshot::new(Player::Playing, at)
        }
    }

    /// A model with `source` as its player and `mock` as LRCLIB (no disk
    /// cache, no retry waits). Returns how many services were started.
    fn with(m: &mut Model, source: &FakeSource, mock: &Mock) -> Arc<AtomicUsize> {
        let source = source.clone();
        m.music.connect_with(
            move || Box::new(source.clone()),
            || {
                crate::media::art::ArtLoader::preloaded(
                    "none",
                    crate::media::art::Art::solid(crate::theme::Rgb(0, 0, 0)),
                )
            },
        );
        let started = Arc::new(AtomicUsize::new(0));
        let (mock, count) = (mock.clone(), Arc::clone(&started));
        m.lyrics.start_with(move || {
            count.fetch_add(1, Ordering::SeqCst);
            LyricsService::spawn(
                Lrclib::with_http(mock.clone(), "http://test"),
                None,
                Vec::new(),
            )
            .ok()
        });
        started
    }

    /// Ticks at `at` until the lookup has answered (real time passes for
    /// the worker thread; the model's clock stays at `at`).
    fn settle(m: &mut Model, at: Instant) {
        for _ in 0..500 {
            tick(m, at);
            if !matches!(m.lyrics.found, Some(Fetch::Looking)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("no answer from the lyrics worker");
    }

    fn placed(name: &str, cols: u16, rows: u16) -> (Model, Instant, FakeSource, Mock) {
        let (mut m, t0) = model_with(Session::default(), temp_config(name), cols, rows);
        let source = FakeSource::new(playing(track("t:1", "Slow Rise"), S * 3, t0), Vec::new());
        let mock = Mock::new([record(LRC)]);
        with(&mut m, &source, &mock);
        (m, t0, source, mock)
    }

    #[test]
    fn off_by_default_asks_nobody() {
        let (mut m, t0) = model("lyrics-off");
        let source = FakeSource::new(playing(track("t:1", "Slow Rise"), S, t0), Vec::new());
        let mock = Mock::new([]);
        let started = with(&mut m, &source, &mock);
        tick(&mut m, t0);
        assert_eq!(m.settings.dock.place(&Lyrics), Place::Off);
        assert_eq!(started.load(Ordering::SeqCst), 0);
        assert!(m.music.snapshot.is_none(), "no player either");
        assert!(mock.urls().is_empty());
    }

    #[test]
    fn placing_it_reads_the_player_and_syncs_the_lines() {
        let (mut m, t0, _source, mock) = placed("lyrics-sync", 120, 36);
        m.update(Action::Place("lyrics"), t0);
        assert_eq!(
            m.toast.as_ref().unwrap().text,
            "lyrics · beside the lamp · song details go to lrclib.net"
        );
        assert_eq!(m.settings.dock.place(&Music), Place::Off, "music stays off");
        settle(&mut m, t0);
        assert!(m.music.snapshot.is_some(), "the player is read for lyrics");
        assert!(matches!(
            m.lyrics.found,
            Some(Fetch::Lyrics(Words::Synced(_)))
        ));
        assert!(mock.urls()[0].contains("/api/get?track_name=Slow%20Rise"));
        assert_eq!(m.lyrics.widest, 11);

        // 3 s in: the intro (before the first line).
        tick(&mut m, t0);
        assert_eq!(m.lyrics.cursor.unwrap().index, None);
        // 2.5 s later: the first line, fading in.
        tick(&mut m, t0 + S * 2 + S / 2);
        let cursor = m.lyrics.cursor.unwrap();
        assert_eq!(cursor.index, Some(0));
        assert!(m.lyrics.fade(m.now) < 1.0, "a line change fades");
        tick(&mut m, t0 + S * 3);
        assert_eq!(m.lyrics.fade(m.now), 1.0);
        assert!(m.layout.placed(3).is_some(), "lyrics in the panel");
        assert_eq!(Lyrics.rank(&m), 2);
        assert_eq!(Lyrics.chip(&m).unwrap().text, "♪ first line");
    }

    #[test]
    fn seeks_cut_and_track_changes_ask_again() {
        let (mut m, t0, source, mock) = placed("lyrics-seek", 120, 36);
        m.update(Action::Place("lyrics"), t0);
        settle(&mut m, t0);
        tick(&mut m, t0 + S * 3);
        // A seek to the third line: no fade.
        source.set(playing(track("t:1", "Slow Rise"), S * 21, t0 + S * 3));
        tick(&mut m, t0 + S * 3);
        let cursor = m.lyrics.cursor.unwrap();
        assert_eq!(cursor.index, Some(3));
        assert!(cursor.seeked);
        assert_eq!(m.lyrics.fade(m.now), 1.0);

        // The next track: looked up afresh; not found here.
        mock.replies.lock().unwrap().extend([status(404), ok("[]")]);
        source.set(playing(track("t:2", "Blob Merge"), S, t0 + S * 4));
        tick(&mut m, t0 + S * 4);
        // The lookup runs on its own thread: on a fast machine its answer
        // can already be in by this tick, so either state is right here.
        assert!(matches!(
            m.lyrics.found,
            Some(Fetch::Looking | Fetch::NotFound)
        ));
        assert!(m.lyrics.cursor.is_none());
        settle(&mut m, t0 + S * 4);
        assert_eq!(m.lyrics.found, Some(Fetch::NotFound));
        let forms = Lyrics.forms(&m, Place::Overlay);
        assert_eq!(forms.len(), 1, "one calm message");
        assert_eq!(Lyrics.rank(&m), 0);
        assert!(Lyrics.chip(&m).is_none());
    }

    #[test]
    fn every_state_has_a_calm_form() {
        let (mut m, t0, source, mock) = placed("lyrics-states", 120, 36);
        m.update(Action::Place("lyrics"), t0);
        m.update(Action::Place("lyrics"), t0); // on the lava
        settle(&mut m, t0);
        let forms = Lyrics.forms(&m, Place::Overlay);
        assert_eq!((forms[0].size.width, forms[0].size.height), (20, 5));

        let instrumental =
            r#"{"trackName":"x","artistName":"y","duration":214.0,"instrumental":true}"#;
        let cases: [(Result<Reply, String>, Fetch); 2] = [
            (ok(instrumental), Fetch::Lyrics(Words::Instrumental)),
            (status(400), Fetch::Offline),
        ];
        for (n, (reply, want)) in cases.into_iter().enumerate() {
            mock.replies.lock().unwrap().push_back(reply);
            let at = t0 + S * (n as u32 + 1);
            source.set(playing(
                track(&format!("t:{n}x"), &format!("song {n}")),
                S,
                at,
            ));
            settle(&mut m, at);
            assert_eq!(m.lyrics.found, Some(want));
            assert_eq!(Lyrics.forms(&m, Place::Overlay).len(), 1);
        }
        source.set(Snapshot::new(
            Player::Unavailable(Unavailable::NotRunning),
            t0,
        ));
        tick(&mut m, t0);
        let forms = Lyrics.forms(&m, Place::Side);
        assert_eq!(forms.len(), 1);
        assert!(forms[0].size.width <= 20);
        assert!(Lyrics.chip(&m).is_none());
    }

    #[test]
    fn plain_lyrics_when_there_is_no_sync() {
        let (mut m, t0) = model_with(Session::default(), temp_config("lyrics-plain"), 120, 36);
        let source = FakeSource::new(playing(track("t:1", "Slow Rise"), S * 100, t0), Vec::new());
        let plain = r#"{"trackName":"Slow Rise","artistName":"The Paraffins","duration":214.0,
            "instrumental":false,"plainLyrics":"one\ntwo\n\nthree","syncedLyrics":null}"#;
        let mock = Mock::new([ok(plain)]);
        with(&mut m, &source, &mock);
        m.update(Action::Place("lyrics"), t0);
        settle(&mut m, t0);
        assert!(matches!(
            m.lyrics.found,
            Some(Fetch::Lyrics(Words::Plain(_)))
        ));
        assert!(m.lyrics.cursor.is_none());
        assert_eq!(Lyrics.forms(&m, Place::Side).len(), 3);
        assert_eq!(Lyrics.rank(&m), 2);
        assert!(Lyrics.chip(&m).is_none(), "no current line to chip");
    }

    #[test]
    fn placing_it_off_stops_the_lookups() {
        let (mut m, t0, _source, _mock) = placed("lyrics-life", 120, 36);
        for _ in 0..3 {
            m.update(Action::Place("lyrics"), t0);
        }
        tick(&mut m, t0);
        assert_eq!(m.settings.dock.place(&Lyrics), Place::Off);
        assert!(m.lyrics.found.is_none());
        assert!(m.music.snapshot.is_none(), "nothing holds the player");
    }

    #[test]
    fn a_frozen_lamp_wakes_for_the_next_line() {
        let (mut m, t0, _source, _mock) = placed("lyrics-frozen", 120, 36);
        m.update(Action::Place("lyrics"), t0);
        settle(&mut m, t0);
        m.update(Action::Freeze, t0);
        m.toast = None;
        tick(&mut m, t0 + S * 3);
        // At 6 s (+ the 150 ms lead) the next line is 3.85 s away; the
        // player check each second comes first.
        let wake = m.idle_until().unwrap() - (t0 + S * 3);
        assert!(wake <= S + Duration::from_millis(20), "{wake:?}");
        tick(&mut m, t0 + S * 7 + S * 85 / 100);
        let wake = m.idle_until().unwrap() - (t0 + S * 7 + S * 85 / 100);
        assert!(wake <= S / 3, "{wake:?}");
    }
}

// --- first-time guidance (lava-1xk.9, .10, .8) ------------------------------

/// A model as on a first start: the welcome card up.
fn first_start(name: &str, cols: u16, rows: u16) -> (Model, Instant, PathBuf) {
    let path = temp_config(name);
    let (mut m, t0) = model_with(Session::default(), path.clone(), cols, rows);
    m.welcome = m.settings.ui.welcome;
    assert!(m.welcome, "on by default");
    (m, t0, path)
}

#[test]
fn the_welcome_card_goes_with_the_first_key_and_stays_gone() {
    let (mut m, t0, path) = first_start("welcome", 80, 24);
    let (_, form) = crate::ui::cards::welcome_card(&m.layout).expect("room for it");
    assert_eq!(form, crate::ui::cards::WelcomeForm::Full);
    let style = m.style;
    m.update(Action::NextStyle, t0);
    assert!(!m.welcome, "dismissed");
    assert_ne!(m.style, style, "and the key still did its thing");
    tick(&mut m, t0 + Duration::from_millis(1100));
    let (again, _) = model_with(Session::default(), path, 80, 24);
    assert!(!again.settings.ui.welcome, "saved");
    // `w` shows it again.
    m.update(Action::Welcome, t0);
    assert!(crate::ui::cards::welcome_shown(&m));
    m.update(Action::Close, t0);
    assert!(!m.welcome);
}

#[test]
fn a_small_window_gets_a_small_card_and_a_tiny_one_waits() {
    let (m, _, _) = first_start("welcome-small", 30, 10);
    let (_, form) = crate::ui::cards::welcome_card(&m.layout).expect("room for it");
    assert_eq!(form, crate::ui::cards::WelcomeForm::Small);
    assert!(crate::ui::cards::guide(&m.layout, &m).is_none());

    let (mut m, t0, _) = first_start("welcome-tiny", 20, 8);
    assert!(crate::ui::cards::welcome_card(&m.layout).is_none());
    let (_, text) = crate::ui::cards::guide(&m.layout, &m).unwrap();
    assert_eq!(text, " ? help · q quit ");
    // Keys don't dismiss what wasn't shown: it waits for a larger window.
    m.update(Action::NextStyle, t0);
    assert!(m.welcome);
    m.relayout(Rect::new(0, 0, 80, 24));
    assert!(crate::ui::cards::welcome_card(&m.layout).is_some());
    // Help (or esc) does, at any size.
    m.relayout(Rect::new(0, 0, 20, 8));
    m.update(Action::Help, t0);
    assert!(!m.welcome);
    assert!(!m.settings.ui.welcome);
}

#[test]
fn a_lamp_only_start_says_where_help_is() {
    let path = temp_config("minimal-start");
    let session = Session {
        minimal: true,
        ..Session::default()
    };
    let (m, _) = model_with(session, path, 80, 24);
    // model_with dismisses the welcome after Model::new: the toast came
    // from the welcome being on. Build one that's already seen it.
    let mut settings = Settings::default();
    settings.ui.welcome = false;
    settings.ui.mode = config::UiMode::Minimal;
    let path = temp_config("minimal-start-2");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, toml::to_string(&settings).unwrap()).unwrap();
    let (seen, _) = model_with(Session::default(), path, 80, 24);
    assert_eq!(seen.toast.as_ref().unwrap().text, MINIMAL_START);
    assert!(m.toast.is_none(), "the welcome card says it then");
}

#[test]
fn small_pickers_say_enter_saves_and_esc_cancels() {
    for (cols, rows) in [(20, 8), (30, 10), (50, 16)] {
        let (mut m, t0) = model_with(Session::default(), temp_config("guide-pick"), cols, rows);
        m.update(Action::StylePicker, t0);
        let Overlay::Picker(p) = m.overlay else {
            panic!()
        };
        let place = picker::placement(m.layout.area, &m.layout, &p).unwrap();
        let text = match place {
            Placement::Inline { guide, .. } => picker::PICKER_GUIDE[guide.unwrap().1],
            Placement::Sheet { guide, .. } => picker::PICKER_GUIDE[guide.unwrap()],
        };
        assert!(
            text.contains("Enter") && text.contains("Esc"),
            "{cols}x{rows}: {text}"
        );
    }
}

#[test]
fn the_face_picker_previews_a_clock_that_isnt_shown_and_leaves_it_off() {
    use crate::ui::cards::{Preview, face_preview};
    for minimal in [false, true] {
        let (mut m, t0) = model("face-preview");
        if minimal {
            m.update(Action::ToggleMinimal, t0);
        } else {
            m.update(Action::Place("clock"), t0);
            m.update(Action::Place("clock"), t0);
            assert_eq!(m.settings.dock.place(&Clock), Place::Off);
        }
        let place = m.settings.dock.place(&Clock);
        let face = m.face.name();
        m.update(Action::FacePicker, t0);
        m.update(Action::Down, t0);
        let area = m.layout.area;
        let (_, preview) = face_preview(area, &m.layout, &m).expect("a preview");
        assert!(matches!(preview, Preview::Face(f) if f.tier != clock::Tier::Text));
        m.update(Action::Close, t0);
        assert_eq!(m.face.name(), face, "esc puts the face back");
        assert_eq!(m.settings.dock.place(&Clock), place, "and the placement");
        assert!(face_preview(area, &m.layout, &m).is_none());
    }
    // With the clock showing its face beside the lamp: no second one.
    let (mut m, t0) = model("face-preview-shown");
    m.update(Action::FacePicker, t0);
    assert!(face_preview(m.layout.area, &m.layout, &m).is_none());
}
