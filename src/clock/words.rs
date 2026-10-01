//! `words`: the time in words, to the nearest five minutes ("it is half
//! past ten"). L is a classic word-clock letter grid with the live words lit
//! and the rest dim; M and S are the sentence, word-wrapped.

use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};

use super::draw::Pen;
use super::{ClockTime, Face, FaceOptions, FaceStyle, Form, Tier};

pub struct Words;

const HOURS: [&str; 12] = [
    "twelve", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven",
];

/// Five-minute slots: (minute words, "past"/"to"/"" relation).
const SLOTS: [(&[&str], &str); 12] = [
    (&[], ""),
    (&["five"], "past"),
    (&["ten"], "past"),
    (&["quarter"], "past"),
    (&["twenty"], "past"),
    (&["twenty-five"], "past"),
    (&["half"], "past"),
    (&["twenty-five"], "to"),
    (&["twenty"], "to"),
    (&["quarter"], "to"),
    (&["ten"], "to"),
    (&["five"], "to"),
];

/// (slot 0–11, hour 0–11) to the nearest five minutes; ":58" rolls into
/// the next hour's "o'clock".
fn rounded(time: ClockTime) -> (usize, usize) {
    let slot = (usize::from(time.minute) + 2) / 5;
    let hour = usize::from(time.hour % 12) + slot / 12;
    let slot = slot % 12;
    // "to" phrases name the coming hour.
    let hour = if slot > 6 { hour + 1 } else { hour };
    (slot, hour % 12)
}

/// The sentence as wrap tokens: `["it", "is", "half", "past", "ten"]`.
fn sentence(slot: usize, hour: usize) -> Vec<&'static str> {
    let (minutes, relation) = SLOTS[slot];
    let mut words = vec!["it", "is"];
    words.extend_from_slice(minutes);
    if slot == 0 {
        words.extend([HOURS[hour], "o'clock"]);
    } else {
        words.extend([relation, HOURS[hour]]);
    }
    words
}

/// Greedy word wrap at `width`. Tokens never split.
fn wrap(words: &[&str], width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for w in words {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + w.len() <= width => {
                line.push(' ');
                line.push_str(w);
            }
            _ => lines.push((*w).to_string()),
        }
    }
    lines
}

/// Wrap widths for the sentence forms, M then S.
const WRAP: [(Tier, usize); 2] = [(Tier::M, 24), (Tier::S, 16)];

/// Fixed sentence-form sizes: the worst case over every phrase, so the size
/// never changes with the time.
fn sentence_size(width: usize) -> Size {
    static SIZES: OnceLock<Vec<(usize, Size)>> = OnceLock::new();
    let sizes = SIZES.get_or_init(|| {
        WRAP.iter()
            .map(|&(_, wrap_w)| {
                let (mut w, mut h) = (0, 0);
                for slot in 0..12 {
                    for hour in 0..12 {
                        let lines = wrap(&sentence(slot, hour), wrap_w);
                        w = w.max(lines.iter().map(String::len).max().unwrap_or(0));
                        h = h.max(lines.len());
                    }
                }
                (wrap_w, Size::new(w as u16, h as u16))
            })
            .collect()
    });
    sizes
        .iter()
        .find(|(w, _)| *w == width)
        .map_or(Size::ZERO, |(_, s)| *s)
}

/// The letter grid (QLOCKTWO layout), letters spaced one apart.
const GRID: [&str; 10] = [
    "itlisasampm",
    "acquarterdc",
    "twentyfivex",
    "halfstenfto",
    "pasterunine",
    "onesixthree",
    "fourfivetwo",
    "eighteleven",
    "seventwelve",
    "tenseoclock",
];
const GRID_SIZE: Size = Size::new(21, 10);

/// Grid spans as (row, col, len).
type Span = (usize, usize, usize);
const IT_IS: [Span; 2] = [(0, 0, 2), (0, 3, 2)];
const AM: Span = (0, 7, 2);
const PM: Span = (0, 9, 2);
const A_QUARTER: [Span; 2] = [(1, 0, 1), (1, 2, 7)];
const TWENTY: Span = (2, 0, 6);
const FIVE_MIN: Span = (2, 6, 4);
const HALF: Span = (3, 0, 4);
const TEN_MIN: Span = (3, 5, 3);
const TO: Span = (3, 9, 2);
const PAST: Span = (4, 0, 4);
const OCLOCK: Span = (9, 5, 6);
const HOUR_SPANS: [Span; 12] = [
    (8, 5, 6), // twelve
    (5, 0, 3), // one
    (6, 8, 3), // two
    (5, 6, 5), // three
    (6, 0, 4), // four
    (6, 4, 4), // five
    (5, 3, 3), // six
    (8, 0, 5), // seven
    (7, 0, 5), // eight
    (4, 7, 4), // nine
    (9, 0, 3), // ten
    (7, 5, 6), // eleven
];

fn lit_spans(time: ClockTime, hour24: bool) -> Vec<Span> {
    let (slot, hour) = rounded(time);
    let mut spans = IT_IS.to_vec();
    match slot {
        0 => spans.push(OCLOCK),
        1 | 11 => spans.push(FIVE_MIN),
        2 | 10 => spans.push(TEN_MIN),
        3 | 9 => spans.extend(A_QUARTER),
        4 | 8 => spans.push(TWENTY),
        5 | 7 => spans.extend([TWENTY, FIVE_MIN]),
        _ => spans.push(HALF),
    }
    match slot {
        0 => {}
        1..=6 => spans.push(PAST),
        _ => spans.push(TO),
    }
    spans.push(HOUR_SPANS[hour]);
    if !hour24 {
        spans.push(if time.hour < 12 { AM } else { PM });
    }
    spans
}

impl Face for Words {
    fn name(&self) -> &'static str {
        "words"
    }

    fn forms(&self, _opts: FaceOptions) -> Vec<Form> {
        let form = |tier, size| Form {
            tier,
            seconds: false,
            meridiem: false,
            size,
        };
        let mut forms = vec![form(Tier::L, GRID_SIZE)];
        forms.extend(WRAP.iter().map(|&(tier, w)| form(tier, sentence_size(w))));
        forms
    }

    fn draw(
        &self,
        form: Form,
        time: ClockTime,
        opts: FaceOptions,
        area: Rect,
        buf: &mut Buffer,
        style: FaceStyle,
    ) {
        let mut pen = Pen::new(buf, area);
        if form.tier == Tier::L {
            let lit = lit_spans(time, opts.hour24);
            for (row, letters) in GRID.iter().enumerate() {
                for (col, ch) in letters.chars().enumerate() {
                    let on = lit
                        .iter()
                        .any(|&(r, c, n)| r == row && (c..c + n).contains(&col));
                    pen.put(col * 2, row, ch, if on { style.main } else { style.dim });
                }
            }
            return;
        }
        let wrap_w = WRAP
            .iter()
            .find(|(t, _)| *t == form.tier)
            .map_or(0, |(_, w)| *w);
        let (slot, hour) = rounded(time);
        for (y, line) in wrap(&sentence(slot, hour), wrap_w).iter().enumerate() {
            pen.text(0, y, line, style.main);
        }
    }
}
