use std::path::PathBuf;

use super::*;

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
    assert!(m.settings.ui.status_bar, "b does nothing in minimal mode");
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
    assert_eq!(scroll, crate::ui::help::max_scroll(m.layout.area));
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

#[test]
fn frame_toast_shows_the_resolved_frame() {
    let (mut m, t0) = model("frame");
    m.update(Action::CycleFrame, t0);
    assert_eq!(m.toast.as_ref().unwrap().text, "glass · glass");
    m.update(Action::CycleFrame, t0);
    assert_eq!(m.toast.as_ref().unwrap().text, "bleed · bleed");
    assert_eq!(m.layout.lamp.unwrap().frame, LampFrame::Bleed);
    m.update(Action::CycleFrame, t0);
    assert_eq!(m.toast.as_ref().unwrap().text, "auto · glass");
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

#[test]
fn corrupt_config_toasts_and_uses_defaults() {
    let path = temp_config("corrupt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "this is = = not toml").unwrap();
    let (m, _) = model_with(Session::default(), path, 80, 24);
    assert_eq!(
        m.toast.as_ref().unwrap().text,
        "config unreadable · using defaults"
    );
    assert_eq!(m.style, StyleId::default());
}

#[test]
fn focus_and_freeze_lower_the_frame_rate() {
    let (mut m, t0) = model("fps");
    assert_eq!(m.target_fps(), 60);
    m.update(Action::Focus(false), t0);
    assert_eq!(m.target_fps(), 10);
    m.update(Action::Focus(true), t0);
    m.update(Action::Freeze, t0);
    assert_eq!(m.target_fps(), 2);
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
    let view = m.layout.lamp.unwrap().view;
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
fn resize_across_the_glass_threshold_switches_the_container() {
    let (mut m, t0) = model("resize");
    assert_eq!(m.layout.lamp.unwrap().frame, LampFrame::Glass);
    m.update(Action::Resize, t0);
    m.tick(
        t0 + Duration::from_millis(16),
        Rect::new(0, 0, 60, 12),
        local(),
    );
    assert_eq!(m.layout.lamp.unwrap().frame, LampFrame::Bleed);
    assert_eq!(m.field.shape(), Shape::Tank);
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
