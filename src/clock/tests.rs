use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::pomodoro::Phase;
use super::*;

const MARGIN: u16 = 2;
const SENTINEL: char = '¤';

fn t(h: u8, m: u8, s: u8) -> ClockTime {
    ClockTime::new(h, m, s).unwrap()
}

fn all_options() -> [FaceOptions; 4] {
    [(true, false), (true, true), (false, false), (false, true)]
        .map(|(hour24, seconds)| FaceOptions { hour24, seconds })
}

/// Render into a `w`×`h` area inset by a sentinel-filled margin. Returns
/// the inner area and buffer; panics if anything was written outside it.
fn render_checked(widget: impl Widget, w: u16, h: u16) -> (Rect, Buffer) {
    let outer = Rect::new(0, 0, w + 2 * MARGIN, h + 2 * MARGIN);
    let mut buf = Buffer::empty(outer);
    for cell in buf.content.iter_mut() {
        cell.set_char(SENTINEL);
    }
    let inner = Rect::new(MARGIN, MARGIN, w, h);
    for pos in inner.positions() {
        buf[pos].set_char(' ');
    }
    widget.render(inner, &mut buf);
    for pos in outer.positions() {
        if !inner.contains(pos) {
            assert_eq!(
                buf[pos].symbol(),
                SENTINEL.to_string(),
                "wrote outside {w}x{h} at {pos:?}"
            );
        }
    }
    (inner, buf)
}

/// Rows of the inner area as strings, right-trimmed.
fn rows(area: Rect, buf: &Buffer) -> Vec<String> {
    area.rows()
        .map(|row| {
            row.positions()
                .map(|p| buf[p].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Bounding size (from the area's top-left) of non-blank cells.
fn ink_extent(area: Rect, buf: &Buffer) -> Size {
    let mut ext = Size::ZERO;
    for p in area.positions() {
        if buf[p].symbol() != " " {
            ext.width = ext.width.max(p.x - area.x + 1);
            ext.height = ext.height.max(p.y - area.y + 1);
        }
    }
    ext
}

fn face_rows(face: &dyn Face, time: ClockTime, opts: FaceOptions, w: u16, h: u16) -> Vec<String> {
    let (area, buf) = render_checked(ClockWidget::new(face, time).options(opts), w, h);
    rows(area, &buf)
}

// ---------------------------------------------------------------- ClockTime

#[test]
fn clock_time_validates_and_converts() {
    assert!(ClockTime::new(24, 0, 0).is_none());
    assert!(ClockTime::new(0, 60, 0).is_none());
    assert!(ClockTime::new(0, 0, 60).is_none());
    assert_eq!(
        ClockTime::from_secs_of_day(14 * 3600 + 32 * 60 + 7),
        t(14, 32, 7)
    );
    assert_eq!(ClockTime::from_secs_of_day(86_400 + 61), t(0, 1, 1));
    assert_eq!(t(0, 5, 0).display_hour(false), 12);
    assert_eq!(t(12, 5, 0).display_hour(false), 12);
    assert_eq!(t(13, 5, 0).display_hour(false), 1);
    assert_eq!(t(13, 5, 0).display_hour(true), 13);
}

// ----------------------------------------------------------------- registry

#[test]
fn registry_names_are_unique_and_cycle() {
    let names: Vec<_> = FACES.iter().map(|f| f.name()).collect();
    assert_eq!(
        names,
        ["blocks", "segment", "analog", "binary", "words", "text"]
    );
    assert_eq!(default_face().name(), "blocks");
    for name in &names {
        assert_eq!(face_by_name(name).unwrap().name(), *name);
    }
    assert!(face_by_name("sundial").is_none());
    assert_eq!(next_face("blocks").name(), "segment");
    assert_eq!(next_face("text").name(), "blocks");
    assert_eq!(next_face("sundial").name(), "blocks");
}

// ------------------------------------------------------------------ sizing

#[test]
fn every_face_bottoms_out_at_text_5x1() {
    for face in FACES {
        for opts in all_options() {
            assert_eq!(face.min_size(opts), Size::new(5, 1), "{}", face.name());
            let fit = face.fit(opts, Size::new(5, 1)).unwrap();
            assert_eq!(fit.tier, Tier::Text);
            assert!(face.fit(opts, Size::new(4, 1)).is_none());
            assert!(face.fit(opts, Size::new(200, 0)).is_none());
        }
    }
}

#[test]
fn forms_are_ordered_largest_first_and_fit_picks_preferred() {
    for face in FACES {
        for opts in all_options() {
            let forms = face.all_forms(opts);
            for pair in forms.windows(2) {
                assert!(
                    pair[0].tier >= pair[1].tier,
                    "{} tiers out of order",
                    face.name()
                );
            }
            let pref = face.preferred_size(opts);
            assert_eq!(face.fit(opts, pref), Some(forms[0]));
        }
    }
}

#[test]
fn degrades_seconds_then_meridiem_then_tier() {
    let opts = FaceOptions {
        hour24: false,
        seconds: true,
    };
    let forms = Blocks.all_forms(opts);
    let xl: Vec<_> = forms
        .iter()
        .filter(|f| f.tier == Tier::XL)
        .map(|f| (f.seconds, f.meridiem))
        .collect();
    assert_eq!(xl, [(true, true), (false, true), (false, false)]);
    // Just too narrow for M with am/pm → bare M.
    let m_bare = forms
        .iter()
        .find(|f| f.tier == Tier::M && !f.meridiem && !f.seconds)
        .unwrap();
    let fit = Blocks.fit(opts, Size::new(m_bare.size.width, 3)).unwrap();
    assert_eq!(fit, *m_bare);
    // Two rows: no block form fits, so text, and the widest text that fits.
    assert_eq!(
        Blocks.fit(opts, Size::new(80, 2)).unwrap().size,
        Size::new(11, 1)
    );
    assert_eq!(
        Blocks.fit(opts, Size::new(10, 2)).unwrap().size,
        Size::new(8, 1)
    );
    assert_eq!(
        Blocks.fit(opts, Size::new(7, 2)).unwrap().size,
        Size::new(5, 1)
    );
}

/// The size table in the lava-ef7 report: HH:MM, 24h.
#[test]
fn face_size_table() {
    let opts = FaceOptions::default();
    let sizes = |face: &dyn Face| -> Vec<(Tier, (u16, u16))> {
        face.all_forms(opts)
            .iter()
            .map(|f| (f.tier, (f.size.width, f.size.height)))
            .collect()
    };
    use Tier::*;
    assert_eq!(
        sizes(&Blocks),
        [(XL, (51, 8)), (L, (34, 5)), (M, (17, 3)), (Text, (5, 1))]
    );
    assert_eq!(
        sizes(&Segment),
        [(XL, (33, 7)), (L, (21, 5)), (M, (17, 3)), (Text, (5, 1))]
    );
    assert_eq!(
        sizes(&Analog),
        [(XL, (31, 16)), (L, (23, 12)), (M, (15, 8)), (Text, (5, 1))]
    );
    assert_eq!(sizes(&Binary), [(M, (12, 6)), (S, (9, 4)), (Text, (5, 1))]);
    assert_eq!(
        sizes(&Words),
        [(L, (21, 10)), (M, (24, 2)), (S, (16, 3)), (Text, (5, 1))]
    );
    assert_eq!(sizes(&super::Text), [(Text, (5, 1))]);
}

/// Every face, every option set, every size from 1×1 up: never panics,
/// never writes outside its rect, and its ink stays inside the chosen form.
#[test]
fn faces_render_cleanly_at_every_size() {
    let sizes = (1..=40u16)
        .flat_map(|w| (1..=20u16).map(move |h| (w, h)))
        .chain(
            (1..=200u16)
                .step_by(13)
                .flat_map(|w| (1..=60u16).step_by(6).map(move |h| (w, h))),
        )
        .chain([(200, 60), (1, 60), (200, 1)]);
    let time = t(23, 58, 59);
    for (w, h) in sizes {
        for face in FACES {
            for opts in all_options() {
                let widget = ClockWidget::new(*face, time).options(opts);
                let fit = widget.fit(Size::new(w, h));
                let (area, buf) = render_checked(widget, w, h);
                let ink = ink_extent(area, &buf);
                match fit {
                    None => assert_eq!(ink, Size::ZERO, "{} drew at {w}x{h}", face.name()),
                    Some(f) => assert!(
                        ink.width <= f.size.width && ink.height <= f.size.height,
                        "{} {:?} ink {ink:?} exceeds {:?}",
                        face.name(),
                        f.tier,
                        f.size
                    ),
                }
            }
        }
    }
}

/// Form sizes are time-independent: times across the whole day (every
/// hour's first and last minute, plus a 7-minute stride) stay inside.
#[test]
fn every_time_of_day_fits_every_form() {
    for face in FACES {
        for opts in all_options() {
            for form in face.all_forms(opts) {
                let minutes = (0..1440)
                    .step_by(7)
                    .chain((0..24).flat_map(|h| [h * 60, h * 60 + 59]));
                for minute in minutes {
                    let time = ClockTime::from_secs_of_day(minute * 60 + 37);
                    let (area, buf) = render_checked(
                        ClockWidget::new(*face, time).options(opts),
                        form.size.width,
                        form.size.height,
                    );
                    let ink = ink_extent(area, &buf);
                    assert!(ink.width <= form.size.width && ink.height <= form.size.height);
                }
            }
        }
    }
}

#[test]
fn alignment_places_the_form() {
    let row = |alignment| {
        let (area, buf) = render_checked(
            ClockWidget::new(&Text, t(9, 5, 0)).alignment(alignment),
            12,
            1,
        );
        rows(area, &buf).remove(0)
    };
    assert_eq!(row(Alignment::Left), "09:05");
    assert_eq!(row(Alignment::Center), "   09:05"); // 7 spare: 3 left, 4 right
    assert_eq!(row(Alignment::Right), "       09:05");
}

#[test]
fn style_is_applied_from_caller() {
    let style = FaceStyle {
        main: Style::new().fg(Color::Red),
        dim: Style::new().fg(Color::Blue),
    };
    let opts = FaceOptions {
        hour24: false,
        seconds: false,
    };
    let (area, buf) = render_checked(
        ClockWidget::new(&Text, t(14, 32, 0))
            .options(opts)
            .style(style),
        8,
        1,
    );
    assert_eq!(rows(area, &buf), [" 2:32 pm"]);
    assert_eq!(buf[(area.x + 1, area.y)].fg, Color::Red);
    assert_eq!(buf[(area.x + 7, area.y)].fg, Color::Blue);
}

// --------------------------------------------------------------- snapshots

#[test]
fn text_snapshots() {
    let opts = |hour24, seconds| FaceOptions { hour24, seconds };
    assert_eq!(
        face_rows(&Text, t(14, 32, 7), opts(true, false), 20, 1),
        ["14:32"]
    );
    assert_eq!(
        face_rows(&Text, t(14, 32, 7), opts(true, true), 20, 1),
        ["14:32:07"]
    );
    assert_eq!(
        face_rows(&Text, t(14, 32, 7), opts(false, true), 20, 1),
        [" 2:32:07 pm"]
    );
    assert_eq!(
        face_rows(&Text, t(0, 1, 0), opts(false, false), 20, 1),
        ["12:01 am"]
    );
    assert_eq!(
        face_rows(&Text, t(0, 1, 0), opts(false, false), 6, 1),
        ["12:01"]
    );
}

#[test]
fn blocks_m_matches_design_mockup() {
    // design.md §4.5, 14:32
    assert_eq!(
        face_rows(&Blocks, t(14, 32, 0), FaceOptions::default(), 17, 3),
        [
            "▄█  █ █ ▄ ▀▀█ ▀▀█",
            " █  ▀▀█ ▄ ▀▀█ █▀▀",
            "▀▀▀   ▀   ▀▀▀ ▀▀▀"
        ]
    );
}

#[test]
fn blocks_l_is_the_font_doubled() {
    let rows = face_rows(&Blocks, t(14, 32, 0), FaceOptions::default(), 34, 5);
    assert_eq!(rows[0], "  ██    ██  ██      ██████  ██████");
    assert_eq!(rows[4], "██████      ██      ██████  ██████");
}

#[test]
fn blocks_12h_blanks_the_tens_and_adds_meridiem() {
    let opts = FaceOptions {
        hour24: false,
        seconds: false,
    };
    assert_eq!(
        face_rows(&Blocks, t(9, 5, 0), opts, 20, 3),
        [
            "    █▀█ ▄ █▀█ █▀▀",
            "    ▀▀█ ▄ █ █ ▀▀█",
            "    ▀▀▀   ▀▀▀ ▀▀▀ am"
        ]
    );
}

#[test]
fn segment_m_snapshot() {
    assert_eq!(
        face_rows(&Segment, t(14, 32, 0), FaceOptions::default(), 17, 3),
        ["           _   _", "  | |_| ·  _|  _|", "  |   | ·  _| |_"]
    );
}

#[test]
fn segment_l_lights_and_ghosts_segments() {
    let style = FaceStyle {
        main: Style::new().fg(Color::Red),
        dim: Style::new().fg(Color::Blue),
    };
    let (area, buf) = render_checked(ClockWidget::new(&Segment, t(11, 11, 0)).style(style), 21, 5);
    // A "1": the top bar is a ghost, the right verticals are lit.
    assert_eq!(buf[(area.x + 1, area.y)].symbol(), "━");
    assert_eq!(buf[(area.x + 1, area.y)].fg, Color::Blue);
    assert_eq!(buf[(area.x + 3, area.y + 1)].fg, Color::Red);
    assert_eq!(buf[(area.x, area.y + 1)].fg, Color::Blue);
}

#[test]
fn binary_s_snapshot() {
    // 14:32 → columns 1, 4, 3, 2; impossible bits are blank.
    assert_eq!(
        face_rows(&Binary, t(14, 32, 0), FaceOptions::default(), 9, 4),
        ["  ○     ○", "  ●   ○ ○", "○ ○   ● ●", "● ○   ● ○"]
    );
}

#[test]
fn binary_m_labels_columns() {
    let rows = face_rows(&Binary, t(14, 32, 0), FaceOptions::default(), 12, 6);
    assert_eq!(rows[4], "");
    assert_eq!(rows[5], "1  4    3  2");
}

#[test]
fn words_sentences_round_to_five_minutes() {
    let say = |h, m| {
        face_rows(&Words, t(h, m, 0), FaceOptions::default(), 24, 2)
            .join(" ")
            .trim_end()
            .to_string()
    };
    assert_eq!(say(10, 28), "it is half past ten");
    assert_eq!(say(10, 33), "it is twenty-five to eleven");
    assert_eq!(say(10, 58), "it is eleven o'clock");
    assert_eq!(say(23, 58), "it is twelve o'clock");
    assert_eq!(say(0, 14), "it is quarter past twelve");
    assert_eq!(say(15, 0), "it is three o'clock");
}

#[test]
fn words_grid_lights_the_phrase() {
    let style = FaceStyle {
        main: Style::new().fg(Color::Red),
        dim: Style::new().fg(Color::Blue),
    };
    let opts = FaceOptions {
        hour24: false,
        seconds: false,
    };
    let (area, buf) = render_checked(
        ClockWidget::new(&Words, t(22, 15, 0))
            .options(opts)
            .style(style),
        21,
        10,
    );
    let grid = rows(area, &buf);
    assert_eq!(grid[0], "i t l i s a s a m p m");
    let lit: String = area
        .positions()
        .filter(|&p| buf[p].fg == Color::Red)
        .map(|p| buf[p].symbol().to_string())
        .collect();
    assert_eq!(lit, "itispmaquarterpastten");
}

#[test]
fn analog_hands_point_the_right_way() {
    let style = FaceStyle {
        main: Style::new().fg(Color::Red),
        dim: Style::new().fg(Color::Blue),
    };
    // 3:00 — hands go up and right from the centre; nothing lit bottom-left.
    let (area, buf) = render_checked(ClockWidget::new(&Analog, t(3, 0, 0)).style(style), 15, 8);
    let lit = |x: u16, y: u16| buf[(area.x + x, area.y + y)].fg == Color::Red;
    assert!(lit(7, 1), "minute hand should reach the top");
    assert!(lit(10, 3) || lit(10, 4), "hour hand should point right");
    assert!(!lit(3, 6), "nothing lit at seven-thirty");
    // Ticks exist and are dim.
    assert!(area.positions().any(|p| buf[p].fg == Color::Blue));
}

// ---------------------------------------------------------------- pomodoro

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn quick() -> PomodoroConfig {
    PomodoroConfig {
        focus: secs(100),
        short_break: secs(20),
        long_break: secs(60),
        cycles: 4,
        auto_advance: true,
    }
}

#[test]
fn defaults_are_25_5_15_every_4() {
    let c = PomodoroConfig::default();
    assert_eq!(
        (c.focus, c.short_break, c.long_break, c.cycles),
        (secs(1500), secs(300), secs(900), 4)
    );
    let p = Pomodoro::new(c);
    assert_eq!(
        (p.status(), p.phase(), p.cycles()),
        (Status::Idle, Phase::Focus, 0)
    );
    assert_eq!(format_remaining(p.remaining(Instant::now())), "25:00");
}

#[test]
fn idle_does_not_tick_pause_resume_or_skip() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    assert!(!p.pause(t0));
    assert!(!p.resume(t0));
    assert_eq!(p.skip(t0), None);
    assert_eq!(p.tick(t0 + secs(1000)), None);
    assert_eq!(p.remaining(t0 + secs(1000)), secs(100));
    assert_eq!(p.progress(t0), 0.0);
}

#[test]
fn start_runs_focus_and_counts_down() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    assert!(p.start(t0));
    assert!(!p.start(t0 + secs(5)), "start while running is a no-op");
    assert_eq!(p.status(), Status::Running);
    assert_eq!(p.remaining(t0 + secs(30)), secs(70));
    assert!((p.progress(t0 + secs(25)) - 0.25).abs() < 1e-9);
    assert_eq!(p.tick(t0 + secs(99)), None);
}

#[test]
fn pause_banks_elapsed_time() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    assert!(p.pause(t0 + secs(30)));
    assert!(!p.pause(t0 + secs(31)));
    assert_eq!(p.status(), Status::Paused);
    // Time stands still while paused, and tick never ends a paused phase.
    assert_eq!(p.remaining(t0 + secs(500)), secs(70));
    assert_eq!(p.tick(t0 + secs(500)), None);
    assert!(p.resume(t0 + secs(500)));
    assert!(!p.resume(t0 + secs(501)));
    assert_eq!(p.remaining(t0 + secs(510)), secs(60));
    // Second pause/resume accumulates.
    p.pause(t0 + secs(520));
    p.start(t0 + secs(600)); // start resumes when paused
    assert_eq!(p.status(), Status::Running);
    assert_eq!(p.elapsed(t0 + secs(610)), secs(60));
    assert_eq!(p.tick(t0 + secs(649)), None);
    let end = p.tick(t0 + secs(650)).unwrap();
    assert_eq!(end.ended, Phase::Focus);
}

#[test]
fn toggle_starts_pauses_and_resumes() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.toggle(t0);
    assert_eq!(p.status(), Status::Running);
    p.toggle(t0 + secs(10));
    assert_eq!(p.status(), Status::Paused);
    p.toggle(t0 + secs(20));
    assert_eq!(p.status(), Status::Running);
    assert_eq!(p.elapsed(t0 + secs(25)), secs(15));
}

#[test]
fn phase_end_emits_event_and_advances_from_the_deadline() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    let end = p.tick(t0 + secs(103)).unwrap();
    assert_eq!(
        end,
        PhaseEnd {
            ended: Phase::Focus,
            next: Phase::ShortBreak,
            skipped: false
        }
    );
    assert_eq!(p.tick(t0 + secs(103)), None, "an end is reported once");
    assert_eq!(
        (p.phase(), p.status(), p.cycles()),
        (Phase::ShortBreak, Status::Running, 1)
    );
    // The break started at the deadline (t0+100), not at the late tick.
    assert_eq!(p.remaining(t0 + secs(103)), secs(17));
    let end = p.tick(t0 + secs(120)).unwrap();
    assert_eq!((end.ended, end.next), (Phase::ShortBreak, Phase::Focus));
    assert_eq!(p.cycles(), 1, "breaks don't count as cycles");
}

#[test]
fn long_break_every_fourth_focus() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    let mut now = t0;
    let mut seen = Vec::new();
    for _ in 0..16 {
        now += p.remaining(now);
        let end = p.tick(now).unwrap();
        seen.push(end.next);
    }
    use Phase::*;
    assert_eq!(
        seen,
        [
            ShortBreak, Focus, ShortBreak, Focus, ShortBreak, Focus, LongBreak, Focus, //
            ShortBreak, Focus, ShortBreak, Focus, ShortBreak, Focus, LongBreak, Focus,
        ]
    );
    assert_eq!(p.cycles(), 8);
}

#[test]
fn set_progress_for_cycle_dots() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    assert_eq!(p.set_progress(), (0, 4));
    p.start(t0);
    let mut now = t0;
    let mut dots = Vec::new();
    for _ in 0..8 {
        now += p.remaining(now);
        p.tick(now);
        dots.push(p.set_progress().0);
    }
    // After each focus the count goes up; the long break shows a full set,
    // then the next focus starts a fresh one.
    assert_eq!(dots, [1, 1, 2, 2, 3, 3, 4, 0]);
}

#[test]
fn skip_running_paused_and_cadence() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    let end = p.skip(t0 + secs(10)).unwrap();
    assert!(end.skipped);
    assert_eq!(
        (end.ended, end.next, p.cycles()),
        (Phase::Focus, Phase::ShortBreak, 1)
    );
    assert_eq!(p.status(), Status::Running);
    assert_eq!(
        p.remaining(t0 + secs(15)),
        secs(15),
        "next phase starts at the skip"
    );

    p.pause(t0 + secs(15));
    let end = p.skip(t0 + secs(16)).unwrap();
    assert_eq!(end.next, Phase::Focus);
    assert_eq!(
        p.status(),
        Status::Paused,
        "skip keeps a paused timer paused"
    );
    assert_eq!(p.remaining(t0 + secs(99)), secs(100));

    // Skipped focuses still move through the set toward the long break.
    p.resume(t0 + secs(20));
    for _ in 0..5 {
        p.skip(t0 + secs(21));
    }
    assert_eq!(p.phase(), Phase::LongBreak);
    assert_eq!(p.cycles(), 4);
}

#[test]
fn reset_returns_to_idle() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    p.skip(t0);
    p.skip(t0);
    p.reset();
    assert_eq!(
        (p.status(), p.phase(), p.cycles()),
        (Status::Idle, Phase::Focus, 0)
    );
    assert_eq!(p.remaining(t0 + secs(50)), secs(100));
    assert_eq!(p.config(), &quick());
}

#[test]
fn without_auto_advance_the_next_phase_waits() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(PomodoroConfig {
        auto_advance: false,
        ..quick()
    });
    p.start(t0);
    let end = p.tick(t0 + secs(100)).unwrap();
    assert_eq!(end.next, Phase::ShortBreak);
    assert_eq!(p.status(), Status::Paused);
    assert_eq!(p.remaining(t0 + secs(1000)), secs(20));
    p.toggle(t0 + secs(1000));
    assert_eq!(p.remaining(t0 + secs(1005)), secs(15));
}

#[test]
fn a_long_gap_ends_only_one_phase() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    // Laptop asleep for an hour: focus ends, break starts fresh at `now`.
    let later = t0 + secs(3600);
    assert_eq!(p.tick(later).unwrap().ended, Phase::Focus);
    assert_eq!(p.tick(later), None);
    assert_eq!(p.remaining(later), secs(20));
}

#[test]
fn degenerate_config_is_safe() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(PomodoroConfig {
        focus: Duration::ZERO,
        cycles: 0,
        ..quick()
    });
    assert_eq!(p.progress(t0), 1.0);
    p.start(t0);
    // cycles 0 behaves as 1: every break is long.
    assert_eq!(p.tick(t0).unwrap().next, Phase::LongBreak);
    assert_eq!(p.set_progress(), (1, 1));
}

#[test]
fn set_config_keeps_elapsed_time() {
    let t0 = Instant::now();
    let mut p = Pomodoro::new(quick());
    p.start(t0);
    p.set_config(PomodoroConfig {
        focus: secs(50),
        ..quick()
    });
    assert_eq!(p.remaining(t0 + secs(20)), secs(30));
    assert!(p.tick(t0 + secs(60)).is_some());
}

#[test]
fn remaining_is_formatted_rounding_up() {
    assert_eq!(format_remaining(secs(1500)), "25:00");
    assert_eq!(format_remaining(Duration::from_millis(1_103_200)), "18:24");
    assert_eq!(format_remaining(Duration::from_millis(1)), "0:01");
    assert_eq!(format_remaining(Duration::ZERO), "0:00");
    assert_eq!(format_remaining(secs(5400)), "1:30:00");
}

// ---------------------------------------------------------- pomodoro widget

fn pomo_rows(p: &Pomodoro, now: Instant, w: u16, h: u16) -> Vec<String> {
    let (area, buf) = render_checked(PomodoroWidget::new(p, now), w, h);
    rows(area, &buf)
}

/// A pomodoro at 18:24 left in the 3rd focus of the set.
fn third_focus(t0: Instant) -> (Pomodoro, Instant) {
    let mut p = Pomodoro::new(PomodoroConfig::default());
    p.start(t0);
    for _ in 0..4 {
        p.skip(t0);
    }
    (p, t0 + Duration::from_millis(396_000))
}

#[test]
fn pomodoro_widget_full_layout() {
    let t0 = Instant::now();
    let (p, now) = third_focus(t0);
    assert_eq!(
        pomo_rows(&p, now, 22, 3),
        ["focus             ●●○○", "18:24", "━━━━━━────────────────"]
    );
}

#[test]
fn pomodoro_widget_degrades() {
    let t0 = Instant::now();
    let (mut p, now) = third_focus(t0);
    // No room for dots, then no label, then the chip.
    assert_eq!(pomo_rows(&p, now, 20, 3)[0], "focus");
    assert_eq!(
        pomo_rows(&p, now, 17, 3),
        ["18:24", "━━━━─────────────", ""]
    );
    assert_eq!(pomo_rows(&p, now, 10, 2), ["18:24", "━━━───────"]);
    assert_eq!(pomo_rows(&p, now, 10, 1), ["▸ 18:24"]);
    assert_eq!(pomo_rows(&p, now, 6, 1), ["18:24"]);
    assert_eq!(pomo_rows(&p, now, 4, 3), ["", "", ""]);
    p.pause(now);
    assert_eq!(pomo_rows(&p, now, 10, 1), ["‖ 18:24"]);
    assert_eq!(pomo_rows(&p, now, 22, 3)[1], "18:24  paused");
    assert_eq!(pomo_rows(&p, now, 12, 3)[0], "18:24");
}

#[test]
fn pomodoro_widget_idle_shows_full_focus_with_empty_bar() {
    let p = Pomodoro::new(PomodoroConfig::default());
    let now = Instant::now();
    assert_eq!(
        pomo_rows(&p, now, 22, 3),
        ["focus             ○○○○", "25:00", "──────────────────────"]
    );
    assert_eq!(pomo_rows(&p, now, 22, 1), ["25:00"]);
}

#[test]
fn pomodoro_widget_never_overflows() {
    let t0 = Instant::now();
    let (running, now) = third_focus(t0);
    let mut paused = running.clone();
    paused.pause(now);
    let mut long = running.clone();
    for _ in 0..3 {
        long.skip(now);
    }
    assert_eq!(long.phase(), Phase::LongBreak);
    let idle = Pomodoro::new(PomodoroConfig::default());
    for p in [&running, &paused, &long, &idle] {
        for w in 1..=60 {
            for h in 1..=6 {
                render_checked(PomodoroWidget::new(p, now), w, h);
            }
        }
        render_checked(PomodoroWidget::new(p, now), 200, 60);
    }
}

/// Eyeball gallery: `cargo test gallery -- --ignored --nocapture`.
#[test]
#[ignore]
fn gallery() {
    for face in FACES {
        for opts in [
            FaceOptions::default(),
            FaceOptions {
                hour24: false,
                seconds: true,
            },
        ] {
            for form in face.forms(opts) {
                let (area, buf) = render_checked(
                    ClockWidget::new(*face, t(14, 32, 7)).options(opts),
                    form.size.width,
                    form.size.height,
                );
                println!(
                    "{} {:?} {}x{} {:?}",
                    face.name(),
                    form.tier,
                    form.size.width,
                    form.size.height,
                    opts
                );
                for row in rows(area, &buf) {
                    println!("  |{row}");
                }
            }
        }
    }
}
