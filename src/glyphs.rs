//! The symbols the widgets and chrome draw beyond plain text, in two sets:
//! [`RICH`], and [`SAFE`] for terminals that can't be trusted with more
//! (hosts embedding Ghostty's terminal, e.g. Ghostex, drew none of
//! `◂◂ ‖ ▸▸ ≡ ♡ ♥ ⇄ ↻ ♪`: lava-1xk.21, lava-1xk.29). `Model::glyphs` picks
//! one (`cells::safe_glyphs`, `LAVATUI_GLYPHS=safe|rich`). A symbol outside
//! ASCII, Latin-1, `▶ … ━ ─` and the block elements goes in here, never
//! straight into a widget.

/// The set of symbols to draw with.
#[derive(Debug, PartialEq, Eq)]
pub struct Glyphs {
    pub playing: &'static str,
    pub paused: &'static str,
    pub stopped: &'static str,
    pub previous: &'static str,
    pub next: &'static str,
    /// Liked (in `accent`) and not (`dim`).
    pub liked: &'static str,
    pub unliked: &'static str,
    pub add: &'static str,
    pub playlists: &'static str,
    pub shuffle: &'static str,
    pub repeat: &'static str,
    /// Before a message (with its space), or nothing.
    pub note: &'static str,
    /// The toasts for liking and unliking.
    pub liked_toast: &'static str,
    pub unliked_toast: &'static str,
    /// A running timer (the pomodoro).
    pub running: &'static str,
    /// The cursor's row in a list (with its space).
    pub pointer: &'static str,
    /// A playlist that has the playing song already (the add picker).
    pub has: &'static str,
}

/// The usual set.
pub const RICH: Glyphs = Glyphs {
    playing: "▶",
    paused: "‖",
    stopped: "■",
    previous: "◂◂",
    next: "▸▸",
    liked: "♥",
    unliked: "♡",
    add: "+",
    playlists: "≡",
    shuffle: "⇄",
    repeat: "↻",
    note: "♪ ",
    liked_toast: "♥ liked",
    unliked_toast: "♡ unliked",
    running: "▸",
    pointer: "▸ ",
    has: "✓",
};

/// ASCII and Latin-1, and `▶` (which those hosts drew fine).
pub const SAFE: Glyphs = Glyphs {
    playing: "▶",
    paused: "||",
    stopped: "#",
    previous: "«",
    next: "»",
    liked: "<3",
    unliked: "<3",
    add: "+",
    playlists: "=",
    shuffle: "shuf",
    repeat: "rep",
    note: "",
    liked_toast: "liked",
    unliked_toast: "unliked",
    running: "»",
    pointer: "» ",
    has: "*",
};

#[cfg(test)]
mod tests {
    use super::*;

    /// What the hosts were seen to draw.
    pub fn drawable(c: char) -> bool {
        (c as u32) < 0x100 || "▶…━─".contains(c)
    }

    #[test]
    fn the_safe_set_is_drawable_and_the_same_shape() {
        let all = |g: &Glyphs| {
            [
                g.playing,
                g.paused,
                g.stopped,
                g.previous,
                g.next,
                g.liked,
                g.unliked,
                g.add,
                g.playlists,
                g.shuffle,
                g.repeat,
                g.note,
                g.liked_toast,
                g.unliked_toast,
                g.running,
                g.pointer,
                g.has,
            ]
        };
        for s in all(&SAFE) {
            assert!(s.chars().all(drawable), "{s:?}");
        }
        // Markers keep their trailing space, so text lines up either way.
        for (rich, safe) in all(&RICH).into_iter().zip(all(&SAFE)) {
            if safe.is_empty() {
                continue;
            }
            assert_eq!(rich.ends_with(' '), safe.ends_with(' '), "{rich:?}");
        }
        assert_eq!(RICH.pointer.chars().count(), SAFE.pointer.chars().count());
    }
}
