//! Pictures in real pixels. Three protocols, the best the terminal has
//! ([`detect`]): the kitty graphics protocol with Unicode placeholders
//! (kitty, Ghostty; this module), else the iTerm2 inline image protocol
//! (iTerm2, WezTerm, mintty, Rio) or sixel (foot, mlterm, Konsole,
//! Contour), both placed at the cursor ([`inline`]).
//!
//! An image is transmitted once (`a=T`, PNG, chunked base64) with a
//! *virtual* placement (`U=1`) of `cols × rows` cells, and then shown by
//! writing placeholder cells: U+10EEEE plus two combining marks for the
//! cell's row and column in the image, in a foreground colour that *is* the
//! image id. To ratatui those are ordinary one-column cells, so they diff
//! like text: the 60 fps lamp around a cover never rewrites it, moving the
//! cover is just drawing its cells elsewhere, and hiding it is drawing
//! something else there. Nothing is ever placed by cursor position, so
//! nothing can flicker or smear.
//!
//! [`Kitty`] keeps what the terminal holds. Each frame the model says which
//! picture it wants shown ([`Kitty::want`]); a new one (another track, or
//! another size) is queued as chunks and written over the next frames,
//! [`BUDGET`] bytes at a time, inside the frame's synchronized update
//! ([`Kitty::write`]). Until it's all there the cover is drawn in text
//! cells; then the placeholders switch to its id. Two ids alternate, so
//! the switch is a change of every placeholder cell (no reliance on the
//! terminal re-reading a replaced image), and the old image is deleted
//! the frame after. Leaving (off, exit, a panic) deletes ours by id.
//!
//! Pure apart from [`detect`] (environment variables): bytes go to
//! whatever writer the app hands over, so tests read them back.

pub mod inline;
mod sixel;

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The placeholder character (a private-use code point kitty reserved).
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Bytes of image data written per frame at most, so a new cover never
/// stalls a frame of the lamp (a ~300 KB cover takes a few frames).
pub const BUDGET: usize = 96 * 1024;

/// Base64 characters per chunk (the protocol's limit).
const CHUNK: usize = 4096;

/// Rows and columns a placeholder can name (the first entries of kitty's
/// `rowcolumn-diacritics.txt`); bigger pictures are capped to this.
pub const MAX_CELLS: u16 = DIACRITICS.len() as u16;

static DIACRITICS: [char; 128] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}', '\u{483}', '\u{484}',
    '\u{485}', '\u{486}', '\u{487}', '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}',
    '\u{598}', '\u{599}', '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}',
    '\u{5A8}', '\u{5A9}', '\u{5AB}', '\u{5AC}', '\u{5AF}', '\u{5C4}', '\u{610}', '\u{611}',
    '\u{612}', '\u{613}', '\u{614}', '\u{615}', '\u{616}', '\u{617}', '\u{657}', '\u{658}',
    '\u{659}', '\u{65A}', '\u{65B}', '\u{65D}', '\u{65E}', '\u{6D6}', '\u{6D7}', '\u{6D8}',
    '\u{6D9}', '\u{6DA}', '\u{6DB}', '\u{6DC}', '\u{6DF}', '\u{6E0}', '\u{6E1}', '\u{6E2}',
    '\u{6E4}', '\u{6E7}', '\u{6E8}', '\u{6EB}', '\u{6EC}', '\u{730}', '\u{732}', '\u{733}',
    '\u{735}', '\u{736}', '\u{73A}', '\u{73D}', '\u{73F}', '\u{740}', '\u{741}', '\u{743}',
    '\u{745}', '\u{747}', '\u{749}', '\u{74A}', '\u{7EB}', '\u{7EC}', '\u{7ED}', '\u{7EE}',
    '\u{7EF}', '\u{7F0}', '\u{7F1}', '\u{7F3}', '\u{816}', '\u{817}', '\u{818}', '\u{819}',
    '\u{81B}', '\u{81C}', '\u{81D}', '\u{81E}', '\u{81F}', '\u{820}', '\u{821}', '\u{822}',
    '\u{823}', '\u{825}', '\u{826}', '\u{827}', '\u{829}', '\u{82A}', '\u{82B}', '\u{82C}',
];

/// Whether a drawn cell is one of our placeholders: it must reach the
/// terminal exactly as drawn (its colour is the image id).
pub fn is_placeholder(symbol: &str) -> bool {
    symbol.starts_with(PLACEHOLDER)
}

/// A way to show real pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Kitty graphics with Unicode placeholders ([`Kitty`]).
    Kitty,
    /// iTerm2's inline images (`OSC 1337 ; File=`), [`inline`].
    Iterm,
    /// DEC sixel, [`inline`].
    Sixel,
}

/// The pixel protocol this terminal speaks, from its environment (`var`
/// reads one variable; no query, so nothing can block), best first:
///
/// * `LAVATUI_GRAPHICS` = `kitty` / `iterm` / `sixel` / `none` says so
///   outright (for terminals that can't be told apart, e.g. xterm built
///   with sixel).
/// * None inside tmux or screen (they'd need passthrough).
/// * Kitty: kitty and Ghostty (`TERM`, `TERM_PROGRAM`, their own
///   variables). Not WezTerm or Konsole: kitty graphics, but no
///   placeholders.
/// * iTerm2: iTerm2 (`TERM_PROGRAM=iTerm.app`, or `LC_TERMINAL=iTerm2`,
///   which survives ssh), WezTerm, mintty, Rio.
/// * Sixel: foot (`TERM=foot*`), mlterm (`TERM=mlterm*` or `MLTERM`),
///   Konsole 22.04 or later (`KONSOLE_VERSION`), Contour
///   (`TERMINAL_NAME=contour`).
pub fn detect(var: impl Fn(&str) -> Option<String>) -> Option<Protocol> {
    let forced = var("LAVATUI_GRAPHICS").map(|v| v.trim().to_lowercase());
    match forced.as_deref() {
        Some("kitty") => return Some(Protocol::Kitty),
        Some("iterm" | "iterm2") => return Some(Protocol::Iterm),
        Some("sixel") => return Some(Protocol::Sixel),
        Some("none" | "off" | "text") => return None,
        _ => {}
    }
    if var("TMUX").is_some() || var("STY").is_some() {
        return None;
    }
    let term = var("TERM").unwrap_or_default();
    let program = var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let kitty = term == "xterm-kitty"
        || term == "xterm-ghostty"
        || program == "ghostty"
        || program == "kitty"
        || var("KITTY_WINDOW_ID").is_some()
        || var("GHOSTTY_RESOURCES_DIR").is_some();
    let iterm = matches!(program.as_str(), "iterm.app" | "wezterm" | "mintty" | "rio")
        || var("LC_TERMINAL").is_some_and(|t| t == "iTerm2");
    let konsole = var("KONSOLE_VERSION")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .is_some_and(|v| v >= 220_400);
    let sixel = term.starts_with("foot")
        || term.starts_with("mlterm")
        || var("MLTERM").is_some()
        || konsole
        || var("TERMINAL_NAME").is_some_and(|t| t == "contour");
    if kitty {
        Some(Protocol::Kitty)
    } else if iterm {
        Some(Protocol::Iterm)
    } else if sixel {
        Some(Protocol::Sixel)
    } else {
        None
    }
}

/// A picture as transmitted: the source and the cells it fills. Another
/// size is another transmission (the terminal scales to the cells).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub source: String,
    pub cols: u16,
    pub rows: u16,
}

/// Our first image id, once any is used (for [`cleanup`] on any way out,
/// panics included); 0 before.
static USED: AtomicU32 = AtomicU32::new(0);

/// The protocol state: what the terminal has, and what's on its way.
#[derive(Debug)]
pub struct Kitty {
    /// Our two ids: high bytes from the process id, so another program in
    /// the same terminal is unlikely to share them.
    ids: [u32; 2],
    /// Fully transmitted, and the id it went under.
    shown: Option<(Key, u32)>,
    /// On its way: the chunks still to write.
    sending: Option<(Key, u32)>,
    chunks: VecDeque<Vec<u8>>,
    /// An id to delete at the next write (the image just replaced).
    delete: Option<u32>,
}

impl Default for Kitty {
    fn default() -> Self {
        Self::new(std::process::id())
    }
}

impl Kitty {
    pub fn new(pid: u32) -> Self {
        // 24 bits (the id rides in an RGB colour), never 0.
        let base = (pid & 0xffff) << 8;
        Self {
            ids: [base | 1, base | 2],
            shown: None,
            sending: None,
            chunks: VecDeque::new(),
            delete: None,
        }
    }

    /// The id to draw `key`'s placeholders with, once it's all there.
    pub fn ready(&self, key: &Key) -> Option<u32> {
        self.shown
            .as_ref()
            .filter(|(k, _)| k == key)
            .map(|&(_, id)| id)
    }

    /// Whether bytes are still waiting to be written (frames shouldn't
    /// sleep meanwhile).
    pub fn busy(&self) -> bool {
        !self.chunks.is_empty() || self.delete.is_some()
    }

    /// This frame's wish: show `want` (its key and PNG, base64), or nothing.
    /// A transmission in flight always finishes first (the protocol has no
    /// way to abandon one), so a wish made meanwhile waits a frame or two.
    pub fn want(&mut self, want: Option<(Key, &Arc<String>)>) {
        if self.sending.is_some() {
            return;
        }
        match want {
            Some((key, png)) => {
                if self.shown.as_ref().is_some_and(|(k, _)| *k == key) {
                    return;
                }
                let id = match &self.shown {
                    Some((_, id)) if *id == self.ids[0] => self.ids[1],
                    _ => self.ids[0],
                };
                USED.store(self.ids[0], Ordering::Relaxed);
                self.chunks = transmit(id, key.cols, key.rows, png).into();
                self.sending = Some((key, id));
            }
            None => {
                if let Some((_, id)) = self.shown.take() {
                    self.delete = Some(id);
                }
            }
        }
    }

    /// Write what's due this frame: a deletion, then up to [`BUDGET`]
    /// bytes of chunks (whole chunks; at least one).
    pub fn write(&mut self, out: &mut impl Write) -> io::Result<()> {
        if let Some(id) = self.delete.take() {
            out.write_all(&delete(id))?;
        }
        let mut written = 0;
        while let Some(chunk) = self.chunks.front() {
            if written > 0 && written + chunk.len() > BUDGET {
                break;
            }
            written += chunk.len();
            out.write_all(chunk)?;
            self.chunks.pop_front();
        }
        if self.chunks.is_empty()
            && let Some((key, id)) = self.sending.take()
        {
            // From the next frame its placeholders are drawn; the old
            // image goes the frame after that's written.
            if let Some((_, old)) = self.shown.replace((key, id)) {
                self.delete = Some(old);
            }
        }
        Ok(())
    }
}

/// Transmit `png` (base64) as image `id` with a virtual placement of
/// `cols × rows` cells, in chunks; no replies (`q=2`).
fn transmit(id: u32, cols: u16, rows: u16, png: &str) -> Vec<Vec<u8>> {
    let parts: Vec<&[u8]> = png.as_bytes().chunks(CHUNK).collect();
    let last = parts.len().saturating_sub(1);
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let more = u8::from(i < last);
            let mut seq = if i == 0 {
                format!("\x1b_Ga=T,U=1,f=100,t=d,i={id},c={cols},r={rows},q=2,m={more};")
            } else {
                format!("\x1b_Gq=2,m={more};")
            }
            .into_bytes();
            seq.extend_from_slice(part);
            seq.extend_from_slice(b"\x1b\\");
            seq
        })
        .collect()
}

/// Delete image `id`, its placements and its data.
fn delete(id: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\").into_bytes()
}

/// The bytes that delete every image this process may have sent: written
/// on the way out (normal exit, error or panic). Empty if none was.
pub fn cleanup() -> Vec<u8> {
    match USED.load(Ordering::Relaxed) {
        0 => Vec::new(),
        first => [first, first + 1]
            .iter()
            .flat_map(|&id| delete(id))
            .collect(),
    }
}

/// The foreground colour that names image `id` to the terminal.
pub fn id_color(id: u32) -> Color {
    Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)
}

/// Fill `area` with image `id`'s placeholders, on `bg` (what shows where
/// the picture doesn't reach, if the terminal fits it inside the cells).
pub fn draw(buf: &mut Buffer, area: Rect, id: u32, bg: Color) {
    let fg = id_color(id);
    let area = area.intersection(buf.area);
    let mut symbol = String::with_capacity(12);
    for y in 0..area.height.min(MAX_CELLS) {
        for x in 0..area.width.min(MAX_CELLS) {
            symbol.clear();
            symbol.push(PLACEHOLDER);
            symbol.push(DIACRITICS[usize::from(y)]);
            symbol.push(DIACRITICS[usize::from(x)]);
            buf[(area.x + x, area.y + y)]
                .set_symbol(&symbol)
                .set_fg(fg)
                .set_bg(bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    fn key(cols: u16) -> Key {
        Key {
            source: "https://i.example/a".into(),
            cols,
            rows: cols / 2,
        }
    }

    /// The APC sequences in `bytes`, as text.
    fn commands(bytes: &[u8]) -> Vec<String> {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        text.split("\x1b\\")
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.strip_prefix("\x1b_G")
                    .expect("an APC graphics command")
                    .to_owned()
            })
            .collect()
    }

    fn controls(cmd: &str) -> &str {
        cmd.split(';').next().unwrap()
    }

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn detects_kitty_and_ghostty_but_not_through_tmux() {
        let kitty = |pairs| detect(env(pairs)) == Some(Protocol::Kitty);
        assert!(kitty(&[("TERM_PROGRAM", "ghostty")]));
        assert!(kitty(&[("TERM", "xterm-ghostty")]));
        assert!(kitty(&[("TERM", "xterm-kitty")]));
        assert!(kitty(&[("KITTY_WINDOW_ID", "1")]));
        assert!(!kitty(&[("TERM_PROGRAM", "WezTerm")]));
        assert_eq!(detect(env(&[("TERM_PROGRAM", "Apple_Terminal")])), None);
        assert_eq!(detect(env(&[])), None);
        assert_eq!(
            detect(env(&[("TERM", "xterm-kitty"), ("TMUX", "/tmp/x")])),
            None
        );
    }

    #[test]
    fn detects_iterm2_and_sixel_terminals() {
        use Protocol::*;
        let d = |pairs| detect(env(pairs));
        assert_eq!(d(&[("TERM_PROGRAM", "iTerm.app")]), Some(Iterm));
        assert_eq!(
            d(&[("LC_TERMINAL", "iTerm2"), ("TERM", "xterm-256color")]),
            Some(Iterm)
        );
        assert_eq!(d(&[("TERM_PROGRAM", "WezTerm")]), Some(Iterm));
        assert_eq!(d(&[("TERM_PROGRAM", "mintty")]), Some(Iterm));
        assert_eq!(d(&[("TERM", "foot")]), Some(Sixel));
        assert_eq!(d(&[("TERM", "foot-extra")]), Some(Sixel));
        assert_eq!(d(&[("TERM", "mlterm-256color")]), Some(Sixel));
        assert_eq!(d(&[("MLTERM", "3.9.3")]), Some(Sixel));
        assert_eq!(d(&[("KONSOLE_VERSION", "230804")]), Some(Sixel));
        assert_eq!(d(&[("KONSOLE_VERSION", "211200")]), None, "too old");
        assert_eq!(d(&[("TERMINAL_NAME", "contour")]), Some(Sixel));
        // Kitty beats the rest (Ghostty under an iTerm2 ssh login).
        assert_eq!(
            d(&[("TERM", "xterm-ghostty"), ("LC_TERMINAL", "iTerm2")]),
            Some(Kitty)
        );
        assert_eq!(d(&[("TERM_PROGRAM", "WezTerm"), ("TMUX", "x")]), None);
        assert_eq!(d(&[("TERM", "foot"), ("STY", "x")]), None);
        assert_eq!(
            d(&[("TERM", "xterm-256color")]),
            None,
            "plain xterm: unknown"
        );
        assert_eq!(d(&[("TERM_PROGRAM", "vscode")]), None);
    }

    #[test]
    fn the_environment_can_say_outright() {
        use Protocol::*;
        let d = |pairs| detect(env(pairs));
        assert_eq!(
            d(&[("LAVATUI_GRAPHICS", "sixel"), ("TERM", "xterm")]),
            Some(Sixel)
        );
        assert_eq!(d(&[("LAVATUI_GRAPHICS", "iTerm2")]), Some(Iterm));
        assert_eq!(
            d(&[("LAVATUI_GRAPHICS", "kitty"), ("TMUX", "x")]),
            Some(Kitty)
        );
        assert_eq!(
            d(&[("LAVATUI_GRAPHICS", "none"), ("TERM", "xterm-kitty")]),
            None
        );
        assert_eq!(
            d(&[("LAVATUI_GRAPHICS", "bogus"), ("TERM", "foot")]),
            Some(Sixel),
            "unknown values are ignored"
        );
    }

    #[test]
    fn a_picture_is_sent_once_in_budgeted_chunks_then_shown() {
        let mut k = Kitty::new(0x1234);
        // ~300 KB of base64: several frames' worth.
        let png = Arc::new("A".repeat(300_000));
        k.want(Some((key(24), &png)));
        assert_eq!(k.ready(&key(24)), None);
        let mut frames = Vec::new();
        while k.busy() {
            let mut out = Vec::new();
            k.want(Some((key(24), &png))); // asked again each frame: free
            k.write(&mut out).unwrap();
            assert!(out.len() <= BUDGET + 64, "{}", out.len());
            frames.push(out);
        }
        assert!(frames.len() >= 3, "{}", frames.len());
        let all: Vec<String> = frames.iter().flat_map(|f| commands(f)).collect();
        // The first chunk says everything; the rest only "more".
        assert_eq!(
            controls(&all[0]),
            "a=T,U=1,f=100,t=d,i=1192961,c=24,r=12,q=2,m=1"
        );
        assert!(
            all[1..all.len() - 1]
                .iter()
                .all(|c| controls(c) == "q=2,m=1")
        );
        assert_eq!(controls(all.last().unwrap()), "q=2,m=0");
        let payload: usize = all.iter().map(|c| c.split(';').nth(1).unwrap().len()).sum();
        assert_eq!(payload, 300_000);
        assert_eq!(k.ready(&key(24)), Some(0x12_3401));
        // Nothing more for the same picture, whatever the frame count.
        k.want(Some((key(24), &png)));
        let mut out = Vec::new();
        k.write(&mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_new_picture_alternates_ids_and_deletes_the_old_one_after() {
        let mut k = Kitty::new(7);
        let png = Arc::new("QUJD".to_owned());
        let send = |k: &mut Kitty, cols| {
            k.want(Some((key(cols), &png)));
            let mut out = Vec::new();
            k.write(&mut out).unwrap();
            commands(&out)
        };
        let first = send(&mut k, 24);
        assert_eq!(first.len(), 1);
        assert_eq!(k.ready(&key(24)), Some(0x701));
        // Resized: sent again under the other id; the old one stays until
        // the frame after (its placeholders are on screen this frame).
        let second = send(&mut k, 16);
        assert!(second[0].contains("i=1794,c=16,r=8"), "{second:?}");
        assert_eq!(k.ready(&key(24)), None);
        assert_eq!(k.ready(&key(16)), Some(0x702));
        let mut out = Vec::new();
        k.write(&mut out).unwrap();
        assert_eq!(commands(&out), ["a=d,d=I,i=1793,q=2"]);
        // Hidden: deleted.
        k.want(None);
        let mut out = Vec::new();
        k.write(&mut out).unwrap();
        assert_eq!(commands(&out), ["a=d,d=I,i=1794,q=2"]);
        assert!(!k.busy());
        // Cleanup names both ids.
        assert_eq!(commands(&cleanup()).len(), 2);
    }

    #[test]
    fn a_wish_made_mid_transmission_waits_for_it() {
        let mut k = Kitty::new(1);
        let big = Arc::new("A".repeat(BUDGET * 2));
        k.want(Some((key(24), &big)));
        k.write(&mut Vec::new()).unwrap();
        k.want(None);
        k.want(Some((key(16), &big)));
        while k.busy() {
            k.write(&mut Vec::new()).unwrap();
        }
        assert_eq!(k.ready(&key(24)), Some(0x101), "finished what it began");
        k.want(Some((key(16), &big)));
        assert!(k.busy());
    }

    #[test]
    fn placeholders_are_one_cell_wide_and_carry_the_id() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 4));
        draw(&mut buf, Rect::new(1, 1, 4, 2), 0x123401, Color::Black);
        let cell = &buf[(2, 2)];
        assert!(is_placeholder(cell.symbol()));
        assert_eq!(cell.symbol().width(), 1);
        let marks: Vec<char> = cell.symbol().chars().skip(1).collect();
        assert_eq!(marks, [DIACRITICS[1], DIACRITICS[1]]);
        assert_eq!(cell.fg, Color::Rgb(0x12, 0x34, 0x01));
        assert_eq!(buf[(1, 1)].symbol().chars().nth(2), Some(DIACRITICS[0]));
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(buf[(5, 1)].symbol(), " ");
        // Clipped to the buffer.
        draw(&mut buf, Rect::new(4, 3, 9, 9), 1, Color::Black);
    }
}
