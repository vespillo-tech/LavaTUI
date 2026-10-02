//! The floating-text contrast repro (lava-1xk.31): a seeded lamp with the
//! clock, pomodoro, music and lyrics on the lava, drawn frame after frame
//! at 30 fps, and every floating glyph's contrast measured against what
//! the terminal displays behind it (with see-through cell backgrounds:
//! the cell's background at the window's opacity over a dark desktop).
//!
//! `cargo test --release -- --ignored --nocapture contrast_trace`
//! (`SEED`, `FRAMES`, `STYLE`, `PALETTE`, `CELLS=opaque|translucent|both`,
//! `CSV=<path>` for a per-glyph, per-frame dump). It prints, per mode:
//!
//! - how often a glyph reads below 3 : 1 and 4.5 : 1, and the worst;
//! - `stuck`: glyph-frames below 3 : 1 while the other of light / dark
//!   would read ≥ 4.5 : 1 (the ink lags or never adapts);
//! - `flicker`: a glyph's ink going A → B → A within 4 frames;
//! - `sympathetic`: ink changes of glyphs whose own background barely
//!   moved (luminance within 0.01 over the last 8 frames): a word or line
//!   flipping together because of another glyph.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::render_tests::{draw, local, lyrics};
use crate::app::Model;
use crate::config::store::Store;
use crate::config::{CellsChoice, ColorChoice, Session};
use crate::dock::{Anchor, Backdrop, Look, Place, WIDGETS};
use crate::theme;

/// The modelled window opacity (the user's Ghostty: 0.75).
const OPACITY: f32 = 0.75;

fn env<T: std::str::FromStr>(k: &str, default: T) -> T {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn scene(cols: u16, rows: u16, seed: u64, cells: CellsChoice) -> (Model, Instant) {
    let dir = std::env::temp_dir().join(format!("lavatui-contrast-{seed}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let session = Session {
        color: Some(ColorChoice::Truecolor),
        minimal: true,
        style: std::env::var("STYLE").ok(),
        palette: std::env::var("PALETTE").ok(),
        ..Session::default()
    };
    let t0 = Instant::now();
    let mut m = Model::new(
        &session,
        Store::new(Some(dir.join("config.toml"))),
        Rect::new(0, 0, cols, rows),
        None,
        local(),
        seed,
        t0,
    );
    m.welcome = false;
    m.settings.display.cells = cells;
    lyrics(&mut m, t0, 2, 15);
    let at = [
        ("clock", Anchor::Center),
        ("pomodoro", Anchor::TopRight),
        ("music", Anchor::TopLeft),
        ("lyrics", Anchor::Bottom),
    ];
    for (name, anchor) in at {
        let w = WIDGETS.iter().find(|w| w.name() == name).unwrap();
        m.settings.dock.set(*w, Place::Overlay);
        m.settings.dock.set_anchor(*w, anchor);
    }
    m.tick(t0, Rect::new(0, 0, cols, rows), local());
    (m, t0)
}

/// What the terminal shows for a cell background `c`.
fn shown(c: Color, translucent: bool) -> Option<Color> {
    match c {
        Color::Rgb(r, g, b) if translucent => {
            let f = |v: u8| (f32::from(v) * OPACITY).round() as u8;
            Some(Color::Rgb(f(r), f(g), f(b)))
        }
        Color::Rgb(..) => Some(c),
        _ => None,
    }
}

/// The floating glyph cells this frame: (position, is a block glyph).
fn glyphs(m: &Model, area: Rect) -> Vec<((u16, u16), bool)> {
    let mut scratch = Buffer::empty(area);
    for s in &m.layout.on_lava {
        let look = Look {
            backdrop: Backdrop::Lava,
            align: s.align,
        };
        for p in &s.items {
            WIDGETS[p.widget].draw(m, p.form, p.rect, look, &mut scratch);
        }
    }
    area.positions()
        .filter_map(|p| {
            let c = &scratch[p];
            let s = c.symbol();
            (s != " " && s != "█" && c.bg == theme::TERMINAL_DEFAULT)
                .then_some(((p.x, p.y), s == "▀" || s == "▄"))
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    glyphs: u64,
    below3: u64,
    below45: u64,
    worst: f32,
    stuck: u64,
    changes: u64,
    flicker: u64,
    sympathetic: u64,
}

fn run(cells: CellsChoice, csv: &mut Option<String>) -> Tally {
    let (cols, rows) = (env("COLS", 120u16), env("ROWS", 36u16));
    let seed = env("SEED", 7u64);
    let frames = env("FRAMES", 900u32);
    let translucent = cells == CellsChoice::Translucent;
    let (mut m, t0) = scene(cols, rows, seed, cells);
    let area = Rect::new(0, 0, cols, rows);
    let theme = m.theme.clone();
    let lum = |c: Color| theme.luminance(c);
    let (light, dark) = theme.floating_inks();
    let (light_l, dark_l) = (lum(light).unwrap(), lum(dark).unwrap());
    let mut t = Tally {
        worst: f32::MAX,
        ..Tally::default()
    };
    // Per cell: the last few inks and the last background luminance.
    let mut history: HashMap<(u16, u16), (Vec<Color>, Vec<f32>)> = HashMap::new();
    for f in 0..frames {
        let now = t0 + Duration::from_millis(u64::from(f) * 1000 / 30);
        m.tick(now, area, local());
        let buf = draw(&m, cols, rows);
        let mut quiet_changes = 0;
        for (pos, _block) in glyphs(&m, area) {
            let cell = &buf[pos];
            let (Some(fg), Some(bg)) = (lum(cell.fg), shown(cell.bg, translucent).and_then(lum))
            else {
                continue;
            };
            let c = theme::contrast(fg, bg);
            t.glyphs += 1;
            t.worst = t.worst.min(c);
            t.below3 += u64::from(c < 3.0);
            t.below45 += u64::from(c < 4.5);
            let best = theme::contrast(light_l, bg).max(theme::contrast(dark_l, bg));
            t.stuck += u64::from(c < 3.0 && best >= 4.5);
            if let Some(out) = csv {
                let _ = writeln!(
                    out,
                    "{f},{},{},{},{c:.2},{best:.2},{:?},{bg:.4}",
                    pos.0,
                    pos.1,
                    cell.symbol(),
                    cell.fg
                );
            }
            let (inks, bgs) = history.entry(pos).or_default();
            bgs.push(bg);
            if bgs.len() > 8 {
                bgs.remove(0);
            }
            let (lo, hi) = bgs
                .iter()
                .fold((f32::MAX, 0f32), |(a, b), &l| (a.min(l), b.max(l)));
            let moved = hi - lo >= 0.01;
            // Own, light or dark: a role ink's own changes (a lyric
            // fading in) aren't ink changes.
            let class = match cell.fg {
                c if c == light => light,
                c if c == dark => dark,
                _ => Color::Reset,
            };
            if inks.last().is_some_and(|&l| l != class) {
                t.changes += 1;
                quiet_changes += u64::from(!moved);
                let n = inks.len();
                if inks[n.saturating_sub(4)..].contains(&class) {
                    t.flicker += 1;
                }
            }
            inks.push(class);
            if inks.len() > 4 {
                inks.remove(0);
            }
        }
        t.sympathetic += quiet_changes;
    }
    t
}

#[test]
#[ignore = "repro harness: cargo test --release -- --ignored --nocapture contrast_trace"]
fn contrast_trace() {
    let modes: &[(&str, CellsChoice)] = match std::env::var("CELLS").as_deref() {
        Ok("opaque") => &[("opaque", CellsChoice::Opaque)],
        Ok("translucent") => &[("translucent", CellsChoice::Translucent)],
        _ => &[
            ("opaque", CellsChoice::Opaque),
            ("translucent", CellsChoice::Translucent),
        ],
    };
    let mut csv = std::env::var("CSV")
        .ok()
        .map(|_| "frame,x,y,glyph,contrast,best,fg,bg\n".to_owned());
    for &(name, cells) in modes {
        let t = run(cells, &mut csv);
        let pct = |n: u64| 100.0 * n as f64 / t.glyphs.max(1) as f64;
        println!(
            "{name:11} glyph-frames {:7}  <3:1 {:5.2}%  <4.5:1 {:5.2}%  worst {:.2}  \
             stuck {:5.2}%  ink changes {:5}  flicker {:4}  sympathetic {:5}",
            t.glyphs,
            pct(t.below3),
            pct(t.below45),
            t.worst,
            pct(t.stuck),
            t.changes,
            t.flicker,
            t.sympathetic,
        );
    }
    if let (Some(out), Ok(path)) = (csv, std::env::var("CSV")) {
        std::fs::write(path, out).unwrap();
    }
}
