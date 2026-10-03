//! Where a lyric line breaks onto rows, so a line is never cut off
//! whatever its script: between words; between the characters of scripts
//! written without spaces (Chinese, Japanese, Korean: [`words::unspaced`],
//! keeping closing punctuation such as `，` `。` with the character before
//! it); and, for a run with no break in it that's wider than the row
//! (Thai, a long word), between grapheme clusters, so a vowel or tone mark
//! stays on its letter.
//!
//! Rows are byte ranges of the line (spaces between rows dropped), worked
//! out as they're asked for: nothing is allocated, so the widget can lay
//! out every line each frame.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::words::unspaced;

/// `text`'s rows of at most `w` columns (a single grapheme wider than `w`
/// alone on its row), as byte ranges, the last of `most` holding the rest
/// (wider than `w` if it doesn't fit: the caller cuts it). A blank line
/// is one empty row.
pub fn rows(text: &str, w: u16, most: u16) -> Rows<'_> {
    Rows {
        text,
        w: usize::from(w.max(1)),
        most: most.max(1),
        at: 0,
        first: true,
    }
}

/// How many rows `text` takes at `w` columns.
pub fn count(text: &str, w: u16) -> usize {
    rows(text, w, u16::MAX).count()
}

/// `text` on one row of `w` columns: the end of what's kept (a byte
/// offset), and whether it's cut (then `…` follows it, within `w`). Cut at
/// a break (after a word or character, and any `,;:` it ends with dropped).
pub fn cut(text: &str, w: u16) -> (usize, bool) {
    let text_end = text.trim_end().len();
    if text[..text_end].width() <= usize::from(w) {
        return (text_end, false);
    }
    let Some((start, end)) = rows(text, w.saturating_sub(1), 2).next() else {
        return (0, true);
    };
    let kept = text[start..end].trim_end_matches([',', ';', ':', ' ']);
    let end = if kept.is_empty() {
        end
    } else {
        start + kept.len()
    };
    // Not even one character fits beside the `…`.
    if text[start..end].width() >= usize::from(w) {
        return (0, true);
    }
    (end, true)
}

/// The iterator [`rows`] returns.
#[derive(Clone, Debug)]
pub struct Rows<'a> {
    text: &'a str,
    w: usize,
    most: u16,
    at: usize,
    first: bool,
}

impl Iterator for Rows<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        let text = self.text;
        let start = self.at + (text.len() - self.at - text[self.at..].trim_start().len());
        let first = std::mem::take(&mut self.first);
        if start >= text.len() {
            self.at = text.len();
            return first.then_some((0, 0));
        }
        if self.most == 1 {
            self.at = text.len();
            return Some((start, text.trim_end().len()));
        }
        self.most -= 1;
        let (end, resume) = self.row_from(start);
        self.at = resume;
        Some((start, start + text[start..end].trim_end().len()))
    }
}

impl Rows<'_> {
    /// One row from `start`: where it ends and where the next one starts.
    fn row_from(&self, start: usize) -> (usize, usize) {
        let text = &self.text[start..];
        let mut used = 0;
        // The last place the row may end (byte offset in `text`).
        let mut last_break: Option<usize> = None;
        let mut prev: Option<char> = None;
        for (i, g) in text.grapheme_indices(true) {
            let first = g.chars().next().unwrap_or(' ');
            if first.is_whitespace() {
                if prev.is_some_and(|p| !p.is_whitespace()) {
                    last_break = Some(i);
                }
                used += g.width();
                prev = Some(first);
                continue;
            }
            if i > 0 && prev.is_some_and(|p| breaks_between(p, first)) {
                last_break = Some(i);
            }
            let gw = g.width();
            if used > 0 && used + gw > self.w {
                let end = last_break.unwrap_or(i);
                return (start + end, start + end);
            }
            used += gw;
            prev = g.chars().last();
        }
        (self.text.len(), self.text.len())
    }
}

/// Whether a row may end between `before` and `after` (neither a space):
/// next to a character of a script without spaces, but not before closing
/// punctuation (`好，`) nor after opening (`「好`).
fn breaks_between(before: char, after: char) -> bool {
    (unspaced(before) || unspaced(after)) && !closing(after) && !opening(before)
}

fn opening(c: char) -> bool {
    matches!(
        c,
        '(' | '[' | '{' | '「' | '『' | '（' | '【' | '〈' | '《' | '〔' | '“' | '‘' | '¿' | '¡'
    )
}

/// Punctuation that belongs with what's before it.
fn closing(c: char) -> bool {
    !c.is_alphanumeric() && !unspaced(c) && !opening(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str, w: u16) -> Vec<&str> {
        rows(text, w, u16::MAX).map(|(a, b)| &text[a..b]).collect()
    }

    /// Nothing lost: the rows hold every non-space character in order.
    fn whole(text: &str, w: u16) {
        let joined: String = split(text, w).concat();
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(strip(&joined), strip(text), "{text:?} at {w}");
    }

    #[test]
    fn spaced_lines_break_between_words() {
        let line = "Cooling at the top and coming down to try again";
        assert_eq!(
            split(line, 20),
            ["Cooling at the top", "and coming down to", "try again"]
        );
        assert_eq!(split("Slow rise", 20), ["Slow rise"]);
        assert_eq!(split("  two  spaces  here ", 9), ["two", "spaces", "here"]);
        assert_eq!(split("", 10), [""]);
        assert_eq!(split("   ", 10), [""]);
        // A word wider than the row: broken inside, never cut off.
        assert_eq!(
            split("Supercalifragilistic is wide", 10),
            ["Supercalif", "ragilistic", "is wide"]
        );
    }

    #[test]
    fn scripts_without_spaces_break_between_characters() {
        // 12 characters, 24 columns: two rows of 20, nothing cut (claude
        // review #2).
        let zh = "我们一起看着蜡慢慢地升起";
        assert_eq!(split(zh, 20), ["我们一起看着蜡慢慢地", "升起"]);
        // Closing punctuation stays with its character.
        let rows = split("我们一起看着蜡慢慢地，升起", 20);
        assert_eq!(rows, ["我们一起看着蜡慢慢", "地，升起"]);
        // Japanese with Latin mixed in.
        let ja = "君の名は yeah 夜明けまで歌おう";
        for w in 2..30 {
            whole(ja, w);
            for row in split(ja, w) {
                assert!(row.width() <= usize::from(w), "{row:?} at {w}");
            }
        }
        whole("사랑해 사랑해 너를 사랑해", 7);
    }

    #[test]
    fn thai_breaks_between_graphemes_keeping_its_marks() {
        // No spaces and no dictionary: broken where the row is full, but
        // never between a letter and its vowel or tone mark.
        let th = "ฉันรักเธอมากกว่าที่คำพูดจะบอกได้";
        for w in 3..20 {
            whole(th, w);
            for (a, b) in rows(th, w, u16::MAX) {
                assert!(th[a..b].width() <= usize::from(w));
                let next = th[b..].chars().next();
                assert!(
                    next.is_none_or(|c| c.to_string().width() > 0),
                    "a mark split off at {w}"
                );
            }
        }
    }

    #[test]
    fn the_last_of_most_rows_holds_the_rest() {
        let line = "Cooling at the top and coming down to try again";
        let two: Vec<_> = rows(line, 20, 2).map(|(a, b)| &line[a..b]).collect();
        assert_eq!(two, ["Cooling at the top", "and coming down to try again"]);
        assert_eq!(count(line, 20), 3);
        assert_eq!(count(line, 60), 1);
        // A grapheme wider than the row still makes progress.
        assert_eq!(split("我", 1), ["我"]);
    }

    #[test]
    fn cuts_end_at_a_break() {
        let cut_at = |text: &'static str, w| {
            let (end, cut) = cut(text, w);
            (&text[..end], cut)
        };
        assert_eq!(
            cut_at("Slow rise, slow rise", 30),
            ("Slow rise, slow rise", false)
        );
        assert_eq!(cut_at("Slow rise, slow rise", 12), ("Slow rise", true));
        assert_eq!(cut_at("Unbelievably", 6), ("Unbel", true));
        assert_eq!(cut_at("我们一起看着蜡慢慢地升起", 9), ("我们一起", true));
        for w in 2..48 {
            for text in [
                "Floating like a thought behind your eyes",
                "我们一起看着蜡，慢慢地升起",
            ] {
                let (end, cut) = cut(text, w);
                let shown = text[..end].width() + usize::from(cut);
                assert!(shown <= usize::from(w), "{text:?} at {w}");
            }
        }
    }
}
