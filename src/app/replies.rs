//! Terminal replies that crossterm doesn't know, kept away from the keymap.
//!
//! A shell prompt or plugin that queries the terminal (colours, DA,
//! XTGETTCAP, kitty graphics) just before lavatui starts gets its answer
//! after we're reading. crossterm parses none of these, and what it makes
//! of the bytes looks like typing:
//!
//! * DCS `ESC P … ST`, OSC `ESC ] … BEL|ST`, APC `ESC _ … ST` (and SOS
//!   `ESC X`, PM `ESC ^`): alt-P / alt-] / alt-_, the body as plain keys,
//!   then alt-\ (ST) or ctrl-g (BEL). A DCS reply `ESC P + q …` would quit.
//! * CSI with a private marker it has no table for (`ESC [ > 0;95;0c`, the
//!   DA2 reply; `ESC [ =`): it drops `ESC [ >` and the rest arrives as the
//!   keys `0 ; 9 5 ; 0 c`.
//!
//! A reply is written in one go, so crossterm reads it in one go: it's
//! always inside one *burst*, the events already queued together. Filtering
//! whole bursts lets the CSI rule look ahead without delaying a lone key.
//! A string still open at the end of a burst stays open into the next one
//! if it comes within [`STRING_GAP`] (a reply split across reads); after
//! that the next key is the user's again, so a stray alt-] can't eat input.
//!
//! The strings dropped are kept as text too ([`ReplyFilter::take`]): the
//! answers to our own questions (the picture probe, `graphics::probe`).

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// How long an unterminated string reply keeps swallowing the next burst.
pub const STRING_GAP: Duration = Duration::from_millis(100);

/// Longest string reply kept as text (longer ones are cut).
const MAX_REPLY: usize = 1024;

/// Drops terminal replies from bursts of events. Keeps whether a string
/// reply is still open, and since when, and the strings it dropped.
#[derive(Debug, Default)]
pub struct ReplyFilter {
    /// Inside a DCS/OSC/APC string, as of this burst.
    open: Option<Instant>,
    /// The open string so far: its introducer (`P`, `]`, `_`, …), body.
    text: String,
    /// Whole strings dropped, not yet taken.
    done: Vec<String>,
}

impl ReplyFilter {
    /// Remove every event that belongs to a terminal reply from `burst`,
    /// the events read together at `now`.
    pub fn filter(&mut self, burst: &mut Vec<Event>, now: Instant) {
        let mut in_string = self.open.is_some_and(|at| now - at < STRING_GAP);
        if !in_string {
            self.text.clear();
        }
        let mut keep = vec![true; burst.len()];
        let mut i = 0;
        while i < burst.len() {
            let key = key(&burst[i]);
            if in_string {
                keep[i] = false;
                in_string = !key.is_some_and(ends_string);
                if !in_string {
                    self.done.push(std::mem::take(&mut self.text));
                } else if let Some(c) = key.and_then(plain_char)
                    && self.text.len() < MAX_REPLY
                {
                    self.text.push(c);
                }
            } else if let Some(k) = key.filter(|k| starts_string(k)) {
                keep[i] = false;
                in_string = true;
                self.text.clear();
                if let KeyCode::Char(c) = k.code {
                    self.text.push(c);
                }
            } else if let Some(end) = csi_tail(&burst[i..]) {
                keep[i..i + end].fill(false);
                i += end;
                continue;
            }
            i += 1;
        }
        self.open = in_string.then_some(now);
        let mut flags = keep.into_iter();
        burst.retain(|_| flags.next().unwrap_or(true));
    }

    /// The string replies dropped since last time, each as its introducer
    /// and body (`_Gi=31;OK`, `P>|foot(1.18)`, `]10;rgb:…`).
    pub fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.done)
    }
}

fn key(event: &Event) -> Option<&KeyEvent> {
    match event {
        Event::Key(key) => Some(key),
        _ => None,
    }
}

fn plain_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
        {
            Some(c)
        }
        _ => None,
    }
}

/// `ESC P`, `ESC ]`, `ESC _`, `ESC X`, `ESC ^`: crossterm's alt-char.
fn starts_string(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('P' | ']' | '_' | 'X' | '^'))
}

/// ST (`ESC \`, crossterm's alt-\) or BEL (ctrl-g).
fn ends_string(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('\\') => key.modifiers.contains(KeyModifiers::ALT),
        KeyCode::Char('g') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
}

/// The leaked tail of a private-marker CSI reply at the start of
/// `events`: parameters (digits, `;`, `:`) with at least one `;`, then a
/// final byte (`@`..`~`), starting with a digit. Returns how many events
/// it spans. A user's `0` then `c` never matches: keys typed by hand don't
/// share a burst, and they'd need the `;` too.
fn csi_tail(events: &[Event]) -> Option<usize> {
    let first = events.first().and_then(key).and_then(plain_char);
    if !first.is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut separators = 0;
    for (n, event) in events.iter().enumerate() {
        let c = key(event).and_then(plain_char)?;
        match c {
            '0'..='9' | ':' => {}
            ';' => separators += 1,
            '@'..='~' if separators > 0 => return Some(n + 1),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The events crossterm 0.29 makes of `bytes`, written in one go (see
    /// `parse_event`: ESC + byte → alt-key; `ESC [ >` and `ESC [ =` fail to
    /// parse and are dropped; C0 → ctrl-letter).
    pub fn crossterm_events(bytes: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            let event = match b {
                0x1b if bytes.get(i) == Some(&b'[')
                    && matches!(bytes.get(i + 1), Some(b'>' | b'=')) =>
                {
                    i += 2;
                    continue;
                }
                0x1b => {
                    let c = bytes[i] as char;
                    i += 1;
                    let shift = if c.is_ascii_uppercase() {
                        KeyModifiers::SHIFT
                    } else {
                        KeyModifiers::NONE
                    };
                    KeyEvent::new(KeyCode::Char(c), shift | KeyModifiers::ALT)
                }
                0x01..=0x1a => {
                    KeyEvent::new(KeyCode::Char((b - 1 + b'a') as char), KeyModifiers::CONTROL)
                }
                _ => {
                    let c = b as char;
                    let shift = if c.is_ascii_uppercase() {
                        KeyModifiers::SHIFT
                    } else {
                        KeyModifiers::NONE
                    };
                    KeyEvent::new(KeyCode::Char(c), shift)
                }
            };
            out.push(Event::Key(event));
        }
        out
    }

    /// The replies from lava-ebq.31, plus more of the same families.
    pub const REPLIES: &[&[u8]] = &[
        b"\x1bP+q\x1b\\",
        b"\x1bP1+r71=1b5b3f\x1b\\",
        b"\x1bP>|kitty(0.36.0)\x1b\\",
        b"\x1b]11;rgb:0000/0000/0000\x07",
        b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\",
        b"\x1b]10;rgb:ffff/ffff/ffff\x07",
        b"\x1b]4;1;rgb:cdcd/0000/0000\x1b\\",
        b"\x1b[>0;95;0c",
        b"\x1b[>1;10;0c",
        b"\x1b[>41;380;0c",
        b"\x1b[=0;0;0c",
        b"\x1b_Gi=1;OK\x1b\\",
        b"\x1b_Gi=31;ENOENT:qq [-]+\x1b\\",
        b"\x1b^private q\x1b\\",
        b"\x1bXsos q[]-+\x1b\\",
    ];

    fn filtered(bytes: &[u8]) -> Vec<Event> {
        let mut burst = crossterm_events(bytes);
        ReplyFilter::default().filter(&mut burst, Instant::now());
        burst
    }

    fn ch(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn every_reply_is_dropped_whole() {
        for reply in REPLIES {
            assert_eq!(filtered(reply), [], "{:?}", String::from_utf8_lossy(reply));
        }
    }

    #[test]
    fn keys_around_a_reply_survive() {
        let mut bytes = b"s".to_vec();
        bytes.extend_from_slice(b"\x1bP+q\x1b\\");
        bytes.extend_from_slice(b"]");
        bytes.extend_from_slice(b"\x1b[>0;95;0c");
        bytes.extend_from_slice(b"c");
        assert_eq!(filtered(&bytes), [ch('s'), ch(']'), ch('c')]);
    }

    #[test]
    fn plain_keys_pass() {
        // Lone keys, and runs a user could type, including `0`, `;`, `c`.
        for s in [
            "q", "0", "c", "0c", "[", "]", "-", "+", "1;", ";c", "s0;", "?",
        ] {
            let burst = crossterm_events(s.as_bytes());
            let mut kept = burst.clone();
            ReplyFilter::default().filter(&mut kept, Instant::now());
            assert_eq!(kept, burst, "{s:?}");
        }
        // Esc and alt-keys that open no string pass too.
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let alt_s = Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT));
        let mut burst = vec![esc.clone(), alt_s.clone(), ch('q')];
        ReplyFilter::default().filter(&mut burst, Instant::now());
        assert_eq!(burst, [esc, alt_s, ch('q')]);
    }

    #[test]
    fn dropped_strings_are_kept_as_text() {
        let mut f = ReplyFilter::default();
        let mut burst = crossterm_events(b"\x1b_Gi=31;OK\x1b\\x\x1bP>|foot(1.18.1)\x1b\\");
        f.filter(&mut burst, Instant::now());
        assert_eq!(burst, [ch('x')]);
        let mut fence = crossterm_events(b"\x1b]10;rgb:ffff/ffff/ffff\x07");
        f.filter(&mut fence, Instant::now());
        assert_eq!(
            f.take(),
            ["_Gi=31;OK", "P>|foot(1.18.1)", "]10;rgb:ffff/ffff/ffff"]
        );
        assert!(f.take().is_empty());
        // Split across reads: whole once it ends.
        let t0 = Instant::now();
        let mut a = crossterm_events(b"\x1b_Gi=31;");
        f.filter(&mut a, t0);
        assert!(f.take().is_empty());
        let mut b = crossterm_events(b"OK\x1b\\");
        f.filter(&mut b, t0 + Duration::from_millis(5));
        assert_eq!(f.take(), ["_Gi=31;OK"]);
        // A lapsed one is forgotten, not glued onto the next.
        let mut c = crossterm_events(b"\x1b_Gi=31;");
        f.filter(&mut c, t0);
        let mut d = crossterm_events(b"\x1b_Gi=1;OK\x1b\\");
        f.filter(&mut d, t0 + STRING_GAP);
        assert_eq!(f.take(), ["_Gi=1;OK"]);
    }

    #[test]
    fn a_string_split_across_reads_is_still_dropped() {
        let t0 = Instant::now();
        let mut f = ReplyFilter::default();
        let mut first = crossterm_events(b"\x1b]11;rgb:00");
        f.filter(&mut first, t0);
        assert_eq!(first, []);
        let mut rest = crossterm_events(b"00/0000/0000\x07q");
        f.filter(&mut rest, t0 + Duration::from_millis(5));
        assert_eq!(rest, [ch('q')], "the key after BEL is the user's");
    }

    #[test]
    fn an_unterminated_string_lets_go_after_a_gap() {
        // alt-] pressed by hand: the next key, a moment later, still acts.
        let t0 = Instant::now();
        let mut f = ReplyFilter::default();
        let mut alt = crossterm_events(b"\x1b]");
        f.filter(&mut alt, t0);
        assert_eq!(alt, []);
        let mut next = vec![ch('q')];
        f.filter(&mut next, t0 + STRING_GAP);
        assert_eq!(next, [ch('q')]);
    }
}
