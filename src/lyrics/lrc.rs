//! A forgiving LRC parser.
//!
//! ```text
//! [ti:Song]  [ar:Artist]  [offset:+250]       metadata tags (own line)
//! [00:12.34]A line                            mm:ss.xx (also .x, .xxx, :xx, none)
//! [00:40.00][01:20.00]A chorus                one line at several times
//! [00:50.00]                                  empty text: an instrumental gap
//! [01:00.00]<01:00.00>Word <01:00.50>level    enhanced word tags: kept as word times
//! ```
//!
//! Nothing is an error: a line that isn't understood is skipped, so a
//! partly broken file still syncs what it can.
//!
//! Every line comes out with its words timed ([`super::words`]): from the
//! word tags when the file has them, else estimated.

use std::time::Duration;

use super::words::{self, Tag, Timing, Word};

/// One timed line. `text` is empty for a gap (a break between lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub at: Duration,
    pub text: String,
    /// Its words (or characters, in scripts without spaces) and when each
    /// is sung; empty for a gap.
    pub words: Vec<Word>,
    /// When the singing of the line ends (by the next line's start).
    pub end: Duration,
    /// The word times come from the file's word tags, not an estimate.
    pub exact: bool,
}

impl Line {
    pub fn is_gap(&self) -> bool {
        self.text.is_empty()
    }
}

/// The metadata tags this crate reads; the rest are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub by: Option<String>,
    pub length: Option<Duration>,
    /// `[offset:]` in ms, already applied to the line times. Positive means
    /// the lyrics come sooner.
    pub offset_ms: i64,
}

/// Parsed synced lyrics: lines sorted by time (ties keep file order).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Synced {
    pub lines: Vec<Line>,
    pub meta: Meta,
}

impl Synced {
    pub fn parse(src: &str) -> Self {
        let mut meta = Meta::default();
        let mut parsed = Vec::new();
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        for raw in src.lines() {
            parse_line(raw.trim(), &mut meta, &mut parsed);
        }
        let offset = meta.offset_ms;
        if offset != 0 {
            for line in &mut parsed {
                line.at = shift(line.at, offset);
            }
        }
        // Stable: lines sharing a time keep their file order.
        parsed.sort_by_key(|l| l.at);
        Self {
            lines: time_words(parsed),
            meta,
        }
    }
}

/// A line as read, before its words are timed.
struct Parsed {
    at: Duration,
    text: String,
    tags: Vec<Tag>,
}

/// Times every line's words, now that each line's next is known.
fn time_words(parsed: Vec<Parsed>) -> Vec<Line> {
    let timings: Vec<Timing> = (0..parsed.len())
        .map(|i| Timing {
            text: &parsed[i].text,
            at: parsed[i].at,
            next: parsed.get(i + 1).map(|n| n.at),
            tags: &parsed[i].tags,
        })
        .collect();
    let pace = words::pace(&timings);
    let timed: Vec<_> = timings.iter().map(|t| words::time(t, pace)).collect();
    parsed
        .into_iter()
        .zip(timed)
        .map(|(line, (words, end, exact))| Line {
            at: line.at,
            text: line.text,
            words,
            end,
            exact,
        })
        .collect()
}

/// `at` moved `offset_ms` earlier, clamped at zero.
fn shift(at: Duration, offset_ms: i64) -> Duration {
    let ms = i128::try_from(at.as_millis()).unwrap_or(i128::MAX) - i128::from(offset_ms);
    Duration::from_millis(u64::try_from(ms.max(0)).unwrap_or(u64::MAX))
}

fn parse_line(line: &str, meta: &mut Meta, out: &mut Vec<Parsed>) {
    let mut rest = line;
    let mut times = Vec::new();
    while let Some(tag) = rest.strip_prefix('[') {
        let Some(end) = tag.find(']') else { break };
        let body = &tag[..end];
        if let Some(at) = parse_time(body) {
            times.push(at);
        } else if times.is_empty() && tag[end + 1..].trim().is_empty() {
            meta_tag(body, meta);
            return;
        } else {
            // `[00:01.00][Chorus]`: the bracket is part of the text.
            break;
        }
        rest = tag[end + 1..].trim_start();
    }
    if times.is_empty() {
        return;
    }
    let (text, tags) = word_tags(rest);
    let first = times[0];
    out.extend(times.into_iter().map(|at| {
        Parsed {
            at,
            text: text.clone(),
            // Relative to the line's first time, so a repeated line (a chorus
            // with several times) carries them along.
            tags: tags
                .iter()
                .map(|&(pos, t)| Tag {
                    pos,
                    at: t.saturating_sub(first),
                })
                .collect(),
        }
    }));
}

/// `mm:ss`, `mm:ss.f…` or `mm:ss:cc`; minutes may exceed 59.
fn parse_time(body: &str) -> Option<Duration> {
    let mut parts = body.trim().split(':');
    let min = digits(parts.next()?)?;
    let sec_part = parts.next()?;
    let (sec, frac) = match (sec_part.split_once('.'), parts.next()) {
        (Some((s, f)), None) => (s, Some(f)),
        (None, Some(cc)) => (sec_part, Some(cc)),
        (None, None) => (sec_part, None),
        (Some(_), Some(_)) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    let sec = digits(sec)?;
    if sec >= 60 {
        return None;
    }
    let ms = match frac {
        Some(f) => frac_ms(f)?,
        None => 0,
    };
    Some(Duration::from_millis(
        min.checked_mul(60_000)?.checked_add(sec * 1000 + ms)?,
    ))
}

fn digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A decimal fraction of a second as ms: "5" → 500, "05" → 50, "0512" → 51.
fn frac_ms(f: &str) -> Option<u64> {
    if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ms = 0;
    for (i, b) in f.bytes().take(3).enumerate() {
        ms += u64::from(b - b'0') * [100, 10, 1][i];
    }
    Some(ms)
}

fn meta_tag(body: &str, meta: &mut Meta) {
    let Some((key, value)) = body.split_once(':') else {
        return;
    };
    let value = value.trim();
    let text = || (!value.is_empty()).then(|| value.to_string());
    match key.trim().to_ascii_lowercase().as_str() {
        "ti" => meta.title = text(),
        "ar" => meta.artist = text(),
        "al" => meta.album = text(),
        "by" => meta.by = text(),
        "length" => meta.length = parse_time(value),
        "offset" => {
            if let Ok(ms) = value.strip_prefix('+').unwrap_or(value).parse() {
                meta.offset_ms = ms;
            }
        }
        _ => {}
    }
}

/// The text without its enhanced-LRC word timings (`<mm:ss.xx>`; other
/// `<…>` stay) and whitespace runs made one space, plus each timing with
/// where it stood in that text: at the start of the word it times (or at
/// the end of the one before), past the end for the line's end.
fn word_tags(text: &str) -> (String, Vec<(u32, Duration)>) {
    let mut out = String::with_capacity(text.len());
    let mut tags = Vec::new();
    let mut space = false;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(close) = rest.find('>')
            && let Some(at) = parse_time(&rest[1..close])
        {
            let pos = out.len() + usize::from(space && !out.is_empty());
            tags.push((pos as u32, at));
            rest = &rest[close + 1..];
            continue;
        }
        rest = &rest[c.len_utf8()..];
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    (out, tags)
}

/// Plain lyrics as display lines: trimmed, runs of blank lines collapsed to
/// one stanza break, none at the ends.
pub fn plain_lines(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in src.strip_prefix('\u{feff}').unwrap_or(src).lines() {
        let line = line.trim();
        if line.is_empty() && out.last().is_none_or(String::is_empty) {
            continue;
        }
        out.push(line.to_string());
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn timeline(s: &Synced) -> Vec<(u64, &str)> {
        s.lines
            .iter()
            .map(|l| (l.at.as_millis() as u64, l.text.as_str()))
            .collect()
    }

    #[test]
    fn basic_lines_and_timestamp_forms() {
        let s = Synced::parse(
            "[00:01]whole\n[00:02.5]tenths\n[00:03.25]centis\n[00:04.125]millis\n\
             [00:05:50]colon centis\n[01:06.00]minutes\n[120:00.00]long mix\n[00:07.1234]extra digits",
        );
        assert_eq!(
            timeline(&s),
            [
                (1000, "whole"),
                (2500, "tenths"),
                (3250, "centis"),
                (4125, "millis"),
                (5500, "colon centis"),
                (7123, "extra digits"),
                (66_000, "minutes"),
                (7_200_000, "long mix"),
            ]
        );
    }

    #[test]
    fn several_timestamps_on_one_line_and_sorting() {
        let s = Synced::parse("[00:30.00][00:10.00] Chorus \n[00:20.00]Verse\n[00:10.00]same time");
        assert_eq!(
            timeline(&s),
            [
                (10_000, "Chorus"),
                (10_000, "same time"),
                (20_000, "Verse"),
                (30_000, "Chorus")
            ]
        );
    }

    #[test]
    fn metadata_tags_and_offset() {
        let s = Synced::parse(
            "\u{feff}[ti: Song ]\r\n[ar:Artist]\r\n[al:Album]\r\n[by:me]\r\n[length: 03:45]\r\n\
             [re:tool][ve:1.0][#:comment]\r\n[offset:+500]\r\n[00:00.20]early\r\n[00:10.00]later",
        );
        assert_eq!(s.meta.title.as_deref(), Some("Song"));
        assert_eq!(s.meta.artist.as_deref(), Some("Artist"));
        assert_eq!(s.meta.album.as_deref(), Some("Album"));
        assert_eq!(s.meta.by.as_deref(), Some("me"));
        assert_eq!(s.meta.length, Some(ms(225_000)));
        assert_eq!(s.meta.offset_ms, 500);
        // Positive offset: sooner, clamped at 0.
        assert_eq!(timeline(&s), [(0, "early"), (9_500, "later")]);

        let late = Synced::parse("[offset:-250]\n[00:01.00]x");
        assert_eq!(timeline(&late), [(1_250, "x")]);
        // A broken offset is ignored.
        let bad = Synced::parse("[offset:soon]\n[00:01.00]x");
        assert_eq!(timeline(&bad), [(1_000, "x")]);
    }

    #[test]
    fn empty_lines_are_gaps_and_untimed_lines_are_skipped() {
        let s =
            Synced::parse("[00:01.00]a\n[00:05.00]\n\nplain text\n[00:09.00]   \n[00:10.00]b\n");
        assert_eq!(
            timeline(&s),
            [(1000, "a"), (5000, ""), (9000, ""), (10_000, "b")]
        );
        assert!(s.lines[1].is_gap());
    }

    #[test]
    fn word_level_tags_are_stripped() {
        let s = Synced::parse("[00:01.00]<00:01.00>Hello <00:01.50> big <00:02.00>world<00:02.40>");
        assert_eq!(timeline(&s), [(1000, "Hello big world")]);
        // Angle brackets that aren't times stay.
        let s = Synced::parse("[00:01.00]a <3 b <tag> c");
        assert_eq!(s.lines[0].text, "a <3 b <tag> c");
    }

    #[test]
    fn junk_is_skipped_not_fatal() {
        let s = Synced::parse(
            "[00:61.00]bad seconds\n[xx:01.00]bad minutes\n[00:01.ab]bad fraction\n[-00:01.00]negative\n\
             [00:01.00\nunclosed\n[]\n[:]\n[00:02.00][Chorus] kept\n[99999999999999999:00.00]huge\n[00:03.00]ok",
        );
        assert_eq!(timeline(&s), [(2000, "[Chorus] kept"), (3000, "ok")]);
        assert!(Synced::parse("").lines.is_empty());
    }

    #[test]
    fn plain_lines_collapse_blank_runs() {
        assert_eq!(
            plain_lines("\n\n a \n\n\n b\r\nc\n\n"),
            ["a", "", "b", "c"].map(String::from)
        );
        assert!(plain_lines("").is_empty());
    }
}
