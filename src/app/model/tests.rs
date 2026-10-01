use std::path::PathBuf;

use super::*;
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

fn model_with(session: Session, path: PathBuf, cols: u16, rows: u16) -> (Model, Instant) {
    let t0 = Instant::now();
    let model = Model::new(
        &session,
        Store::new(Some(path)),
        Rect::new(0, 0, cols, rows),
        None,
        local(),
        1,
        t0,
    );
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
        "no status bar in minimal · m to leave",
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
    assert_eq!(m.chip_text().unwrap().0, ChipKind::Pomodoro);
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
    assert_eq!(m.toast.as_ref().unwrap().text, "press r again to reset");
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
    assert_eq!(m.toast.as_ref().unwrap().text, "pomodoro reset");
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
    m.tick(t0, Rect::new(0, 0, 40, 60), local());
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
    assert_eq!(m.chip_text().unwrap().1, "▸ 25:00");
    m.update(Action::PomodoroSkip, t0);
    let (kind, text) = m.chip_text().unwrap();
    assert_eq!(kind, ChipKind::Pomodoro);
    assert!(text.starts_with("▸ break "), "{text}");
    m.update(Action::PomodoroToggle, t0);
    assert!(m.chip_text().unwrap().1.starts_with("‖ break "));
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
