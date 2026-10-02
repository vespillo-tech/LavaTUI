//! When each word of a line is sung.
//!
//! Exact when the source says (enhanced LRC's `<mm:ss.xx>` word tags,
//! which [`super::lrc`] keeps as [`Tag`]s); otherwise estimated, which is
//! most of the time: LRCLIB's synced lyrics time lines, not words.
//!
//! The estimate spreads a line's words over the time it is likely sung in,
//! weighted by how long each takes to sing rather than by its letters:
//!
//! - **Weight**: vowel groups (syllables) for alphabetic words, a little
//!   more for long ones; one per character in scripts written without
//!   spaces (Chinese, Japanese, Korean), where each character is a word
//!   here (the unit the highlight moves by).
//! - **Pauses**: punctuation after a word holds it a little longer (a
//!   comma less than a full stop).
//! - **Sung length**: the time to the next line, less a short breath
//!   ([`TAIL`]), but never more than [`STRETCH`] times what the song's own
//!   pace ([`pace`]) gives the line's weight, so a line before a long
//!   instrumental break isn't drawn out over the break. A line is never
//!   still being sung when the next one starts.
//!
//! Partly tagged lines keep their tags and estimate between them.

use std::time::Duration;

/// One word of a line, or one character in scripts without spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    /// Byte range in the line's text.
    pub start: u32,
    pub end: u32,
    /// When it starts being sung.
    pub at: Duration,
}

/// A word tag from the source: the tag at `pos` (a byte offset in the
/// line's text; at or past its end, the line's end) says `at`, relative to
/// the line's own time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub pos: u32,
    pub at: Duration,
}

/// The breath left before the next line: this share of the gap…
const TAIL_SHARE: f32 = 0.12;
/// …at most this long.
const TAIL: f32 = 0.6;
/// A line is sung over at most this many times its weight at the song's
/// pace.
const STRETCH: f32 = 1.5;
/// The song's pace (seconds per unit of weight) is kept within these.
const PACE: (f32, f32) = (0.12, 0.9);
/// The pace when a song has too few lines to tell.
const DEFAULT_PACE: f32 = 0.35;
/// Lines longer than this (to the next) don't count towards the pace.
const PACE_SPAN: f32 = 12.0;

/// A word's place in the line's text, and how long it takes to sing.
#[derive(Clone, Copy, Debug)]
struct Token {
    start: u32,
    end: u32,
    weight: f32,
    /// The pause after it, in the same units.
    pause: f32,
}

/// The line's words: whitespace-separated, with each character of a
/// script written without spaces on its own.
fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut word: Option<usize> = None;
    let push = |out: &mut Vec<Token>, start: usize, end: usize| {
        let s = &text[start..end];
        out.push(Token {
            start: start as u32,
            end: end as u32,
            weight: weight(s),
            pause: pause(s),
        });
    };
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(start) = word.take() {
                push(&mut out, start, i);
            }
        } else if unspaced(c) {
            if let Some(start) = word.take() {
                push(&mut out, start, i);
            }
            push(&mut out, i, i + c.len_utf8());
        } else if word.is_none() {
            // Punctuation right after a character (`好，`) belongs to it.
            match out.last_mut() {
                Some(last) if last.end as usize == i && !c.is_alphanumeric() => {
                    last.end = (i + c.len_utf8()) as u32;
                    last.pause = last.pause.max(pause(&text[i..i + c.len_utf8()]));
                }
                _ => word = Some(i),
            }
        }
    }
    if let Some(start) = word {
        push(&mut out, start, text.len());
    }
    out
}

/// A character of a script written without spaces between words: CJK
/// ideographs, kana, hangul.
fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30ff     // hiragana, katakana
        | 0x3400..=0x4dbf   // CJK extension A
        | 0x4e00..=0x9fff   // CJK unified ideographs
        | 0xac00..=0xd7af   // hangul syllables
        | 0xf900..=0xfaff   // CJK compatibility ideographs
        | 0x20000..=0x2fa1f // CJK extensions B…
    )
}

fn vowel(c: char) -> bool {
    matches!(
        c.to_lowercase().next().unwrap_or(c),
        'a' | 'e' | 'i' | 'o' | 'u' | 'y'
            | 'à'..='æ'
            | 'è'..='ï'
            | 'ò'..='ö'
            | 'ø'..='ü'
            | 'ā'..='ą'
            | 'ē'..='ě'
            | 'ī'..='į'
            | 'ō'..='ő'
            | 'ū'..='ų'
            | 'α' | 'ε' | 'η' | 'ι' | 'ο' | 'υ' | 'ω'
            | 'ά' | 'έ' | 'ή' | 'ί' | 'ό' | 'ύ' | 'ώ'
            | 'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я'
            | 'і' | 'ї' | 'є'
    )
}

/// How long a word takes to sing, in rough syllables.
fn weight(word: &str) -> f32 {
    if word.chars().any(unspaced) {
        return 1.0;
    }
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    let digits = word.chars().filter(char::is_ascii_digit).count();
    if letters == 0 {
        // A number is said ("1999"), a lone mark (`—`, `(x2)`) barely.
        return if digits > 0 { digits as f32 } else { 0.2 };
    }
    let mut groups = 0;
    let mut in_vowel = false;
    let mut last = ' ';
    let mut before_last = ' ';
    for c in word.chars().filter(|c| c.is_alphabetic()) {
        let v =
            vowel(c) && !(c.eq_ignore_ascii_case(&'y') && groups == 0 && !in_vowel && last == ' ');
        if v && !in_vowel {
            groups += 1;
        }
        in_vowel = v;
        before_last = last;
        last = c.to_ascii_lowercase();
    }
    // English's silent final e ("time", "rise"), not "the" or "little".
    if groups > 1 && last == 'e' && !vowel(before_last) && before_last != 'l' {
        groups -= 1;
    }
    let syllables = if groups == 0 {
        // A script whose vowels aren't letters of their own.
        (letters as f32 / 2.5).max(1.0)
    } else {
        groups as f32
    };
    syllables * 0.85 + letters as f32 * 0.05 + digits as f32
}

/// The pause punctuation at the end of a word asks for.
fn pause(word: &str) -> f32 {
    match word
        .trim_end_matches(['"', '\'', ')', '’', '”', '」', '』'])
        .chars()
        .last()
    {
        Some('.' | '!' | '?' | '…' | '。' | '！' | '？') => 0.8,
        Some(',' | ';' | ':' | '-' | '—' | '–' | '，' | '、' | '；') => 0.5,
        _ => 0.0,
    }
}

/// The line's total weight (pauses after its last word don't count).
fn total(tokens: &[Token]) -> f32 {
    let w: f32 = tokens.iter().map(|t| t.weight + t.pause).sum();
    w - tokens.last().map_or(0.0, |t| t.pause)
}

/// One line to time: its text, start, the next line's start, its tags.
pub struct Timing<'a> {
    pub text: &'a str,
    pub at: Duration,
    pub next: Option<Duration>,
    pub tags: &'a [Tag],
}

/// The song's pace: seconds per unit of weight on a typical line, from
/// lines that run straight into the next.
pub fn pace(lines: &[Timing]) -> f32 {
    let mut paces: Vec<f32> = lines
        .iter()
        .filter_map(|l| {
            let span = l.next?.checked_sub(l.at)?.as_secs_f32();
            let weight = total(&tokens(l.text));
            (weight >= 1.0 && span > 0.0 && span <= PACE_SPAN).then(|| span / weight)
        })
        .collect();
    if paces.len() < 3 {
        return DEFAULT_PACE;
    }
    paces.sort_by(f32::total_cmp);
    paces[paces.len() / 2].clamp(PACE.0, PACE.1)
}

/// The line's words with their start times, when the singing ends, and
/// whether every time came from the source.
pub fn time(line: &Timing, pace: f32) -> (Vec<Word>, Duration, bool) {
    let toks = tokens(line.text);
    if toks.is_empty() {
        return (Vec::new(), line.at, false);
    }
    // Tags: the one at a word's start times it; past the text, the end.
    let mut known: Vec<Option<f32>> = vec![None; toks.len()];
    let mut end_tag = None;
    for tag in line.tags {
        let at = tag.at.as_secs_f32();
        if tag.pos as usize >= line.text.len() {
            end_tag = Some(at);
        } else if let Some(i) = toks.iter().position(|t| t.start >= tag.pos)
            && !toks.iter().any(|t| t.start < tag.pos && tag.pos < t.end)
        {
            known[i].get_or_insert(at);
        }
    }
    let exact = known.iter().all(Option::is_some);
    known[0].get_or_insert(0.0);

    // Relative to the line's start from here on, in seconds.
    let room = line.next.map(|n| n.saturating_sub(line.at).as_secs_f32());
    let tail = |room: f32| (room * TAIL_SHARE).min(TAIL);
    let weights: Vec<f32> = toks.iter().map(|t| t.weight).collect();
    // The end: tagged, else the last tag plus the rest at the song's pace,
    // stretched to fill the room (less a breath) up to `STRETCH`.
    let last_known = (0..toks.len())
        .rev()
        .find(|&i| known[i].is_some())
        .unwrap_or(0);
    let mut end = end_tag.unwrap_or_else(|| {
        let from = known[last_known].unwrap_or(0.0);
        let rest = total(&toks[last_known..]);
        match room {
            Some(room) => (room - tail(room)).min(from + rest * pace * STRETCH),
            None => from + rest * pace,
        }
    });
    end = end.max(known[last_known].unwrap_or(0.0));
    if let Some(room) = room {
        end = end.min(room);
    }

    // Between known times, words are spread by weight (pauses included).
    let mut at = vec![0.0f32; toks.len()];
    let mut i = 0;
    while i < toks.len() {
        let from = known[i].unwrap_or(0.0);
        let j = (i + 1..toks.len()).find(|&j| known[j].is_some());
        let (to, upto) = match j {
            Some(j) => (known[j].unwrap_or(end), j),
            None => (end, toks.len()),
        };
        let span: f32 = toks[i..upto]
            .iter()
            .map(|t| t.weight + t.pause)
            .sum::<f32>()
            - if j.is_none() {
                toks[upto - 1].pause
            } else {
                0.0
            };
        let scale = if span > 0.0 {
            (to - from).max(0.0) / span
        } else {
            0.0
        };
        let mut t = from;
        for k in i..upto {
            at[k] = t;
            t += (weights[k] + toks[k].pause) * scale;
        }
        i = upto;
    }

    // Never back in time, never past the end (nor into the next line).
    let mut last = 0.0f32;
    let words = toks
        .iter()
        .zip(at)
        .map(|(tok, t)| {
            last = t.clamp(last, end.max(0.0));
            Word {
                start: tok.start,
                end: tok.end,
                at: line.at + Duration::from_secs_f32(last),
            }
        })
        .collect();
    let end = line.at + Duration::from_secs_f32(end.max(last));
    (words, end, exact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f32) -> Duration {
        Duration::from_secs_f32(s)
    }

    fn words_of(text: &str) -> Vec<&str> {
        tokens(text)
            .iter()
            .map(|t| &text[t.start as usize..t.end as usize])
            .collect()
    }

    fn line<'a>(text: &'a str, at: f32, next: Option<f32>, tags: &'a [Tag]) -> Timing<'a> {
        Timing {
            text,
            at: secs(at),
            next: next.map(secs),
            tags,
        }
    }

    #[test]
    fn words_split_on_spaces_and_per_character_without() {
        assert_eq!(
            words_of("Slow rise, slow rise"),
            ["Slow", "rise,", "slow", "rise"]
        );
        assert_eq!(words_of("你好，世界"), ["你", "好，", "世", "界"]);
        assert_eq!(words_of("君の名は yeah"), ["君", "の", "名", "は", "yeah"]);
        assert_eq!(words_of("사랑해 baby"), ["사", "랑", "해", "baby"]);
        assert!(words_of("   ").is_empty());
    }

    #[test]
    fn weights_follow_syllables_not_letters() {
        let w = |s| weight(s);
        // One syllable, short or long, against three.
        assert!(w("a") < w("strength") && w("strength") < w("beautiful"));
        assert!((w("time") - w("tie")).abs() < 0.1, "silent e");
        assert!(w("little") > w("lit"));
        assert!(w("amber") > w("glow"));
        assert!(w("—") < 0.5);
        assert!(w("1999") >= 4.0);
        // Other alphabets count their vowels too; CJK one per character.
        assert!(w("любовь") > w("да"));
        assert_eq!(w("好"), 1.0);
    }

    #[test]
    fn punctuation_holds_a_word() {
        assert_eq!(pause("rise,"), 0.5);
        assert_eq!(pause("again."), 0.8);
        assert_eq!(pause("\"why?\""), 0.8);
        assert_eq!(pause("slow"), 0.0);
    }

    /// Properties every estimate keeps.
    fn check(words: &[Word], end: Duration, l: &Timing) {
        assert!(!words.is_empty());
        assert_eq!(words[0].at, l.at, "the first word starts the line");
        for pair in words.windows(2) {
            assert!(pair[0].at <= pair[1].at, "monotonic: {words:?}");
            assert!(pair[0].end <= pair[1].start);
        }
        assert!(words.last().unwrap().at <= end);
        if let Some(next) = l.next {
            assert!(end <= next, "{end:?} past the next line {next:?}");
        }
    }

    #[test]
    fn a_line_fills_its_time_less_a_breath() {
        let l = line(
            "Down at the bottom where the warm light grows",
            10.0,
            Some(14.5),
            &[],
        );
        let (words, end, exact) = time(&l, 0.4);
        check(&words, end, &l);
        assert!(!exact);
        let sung = (end - l.at).as_secs_f32();
        assert!((3.9..4.5).contains(&sung), "{sung}");
        // Two-syllable "bottom" gets more time than "at".
        let len = |i: usize| (words[i + 1].at - words[i].at).as_secs_f32();
        assert!(len(3) > len(1) * 1.5, "{words:?}");
    }

    #[test]
    fn a_comma_holds_the_word_before_it() {
        let l = line("rise, slow rise", 0.0, Some(3.0), &[]);
        let (words, end, _) = time(&l, 0.4);
        check(&words, end, &l);
        let rise = words[1].at - words[0].at;
        let slow = words[2].at - words[1].at;
        assert!(rise > slow, "{words:?}");
    }

    #[test]
    fn a_line_before_a_long_break_is_not_drawn_out() {
        let l = line("Nothing in a hurry", 30.0, Some(55.0), &[]);
        let (words, end, _) = time(&l, 0.4);
        check(&words, end, &l);
        let sung = (end - l.at).as_secs_f32();
        assert!(sung < 5.0, "{sung}");
        // The last line, with nothing after it: the song's pace.
        let last = line("Nothing in a hurry", 30.0, None, &[]);
        let (_, end, _) = time(&last, 0.4);
        assert!((end - last.at).as_secs_f32() < 5.0);
    }

    #[test]
    fn a_fast_line_squeezes_into_its_time() {
        let l = line(
            "I said it all and then I said it all again",
            5.0,
            Some(6.0),
            &[],
        );
        let (words, end, _) = time(&l, 0.4);
        check(&words, end, &l);
        assert!(end <= secs(6.0));
    }

    #[test]
    fn tags_are_kept_and_the_untagged_estimated_between() {
        let text = "Hello big wide world";
        let tags = [
            Tag {
                pos: 0,
                at: secs(0.0),
            },
            Tag {
                pos: 6,
                at: secs(0.5),
            },
            // "wide" untagged.
            Tag {
                pos: 15,
                at: secs(2.0),
            },
            Tag {
                pos: 20,
                at: secs(2.6),
            },
        ];
        let l = line(text, 10.0, Some(14.0), &tags);
        let (words, end, exact) = time(&l, 0.4);
        check(&words, end, &l);
        assert!(!exact, "one word untagged");
        assert_eq!(words[1].at, secs(10.5));
        assert!(words[2].at > secs(10.5) && words[2].at < secs(12.0));
        assert_eq!(words[3].at, secs(12.0));
        assert!((end.as_secs_f32() - 12.6).abs() < 1e-3, "{end:?}");
        let all = [
            Tag {
                pos: 0,
                at: secs(0.0),
            },
            Tag {
                pos: 6,
                at: secs(0.5),
            },
            Tag {
                pos: 10,
                at: secs(1.2),
            },
            Tag {
                pos: 15,
                at: secs(2.0),
            },
        ];
        let (_, _, exact) = time(&line(text, 10.0, Some(14.0), &all), 0.4);
        assert!(exact);
    }

    #[test]
    fn broken_tags_never_go_back_or_past_the_next_line() {
        let tags = [
            Tag {
                pos: 0,
                at: secs(0.0),
            },
            Tag {
                pos: 2,
                at: secs(3.0),
            },
            Tag {
                pos: 4,
                at: secs(1.0),
            },
            Tag {
                pos: 6,
                at: secs(99.0),
            },
        ];
        let l = line("aa bb cc dd", 0.0, Some(4.0), &tags);
        let (words, end, _) = time(&l, 0.4);
        check(&words, end, &l);
    }

    #[test]
    fn pace_is_the_songs_typical_line() {
        let lines: Vec<Timing> = (0..9)
            .map(|i| {
                line(
                    "one two three four",
                    i as f32 * 2.0,
                    Some(i as f32 * 2.0 + 2.0),
                    &[],
                )
            })
            .collect();
        let p = pace(&lines);
        assert!((0.4..0.6).contains(&p), "{p}");
        assert_eq!(pace(&lines[..2]), DEFAULT_PACE);
    }
}
