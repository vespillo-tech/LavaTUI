//! Which player to follow when several are open (macOS: Spotify and Apple
//! Music; Linux: MPRIS players; Windows: SMTC sessions).
//!
//! LavaTUI is a controller for whatever is playing, with Spotify first
//! only while it is actually playing. In order:
//!
//! 1. Spotify, if it's playing.
//! 2. The player in use (the one shown), if it's playing: another player
//!    starting up doesn't steal the widget.
//! 3. Any other player that's playing (the one the system calls current
//!    first).
//! 4. Nothing playing: the player in use, so pausing it leaves the keys on
//!    it (play resumes what was just paused, not some other app).
//! 5. The one the system calls current (Windows), then Spotify, then the
//!    first in the backend's order (by name on Linux, so it's stable).
//!
//! An idle Spotify therefore never wins over another app that is playing.

/// One open player, as far as choosing goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Player {
    pub spotify: bool,
    pub playing: bool,
    /// The one the system calls current (Windows' current session; never
    /// on macOS or Linux).
    pub current: bool,
    /// The one followed last.
    pub in_use: bool,
}

impl Player {
    /// Lower is better; see the module docs.
    fn rank(&self) -> u8 {
        match *self {
            Self {
                playing: true,
                spotify: true,
                ..
            } => 0,
            Self {
                playing: true,
                in_use: true,
                ..
            } => 1,
            Self {
                playing: true,
                current: true,
                ..
            } => 2,
            Self { playing: true, .. } => 3,
            Self { in_use: true, .. } => 4,
            Self { current: true, .. } => 5,
            Self { spotify: true, .. } => 6,
            _ => 7,
        }
    }
}

/// The index of the player to follow, the first on a tie; `None` when
/// there are none.
pub fn pick(players: impl IntoIterator<Item = Player>) -> Option<usize> {
    players
        .into_iter()
        .enumerate()
        .min_by_key(|(i, p)| (p.rank(), *i))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPOTIFY: Player = Player {
        spotify: true,
        playing: false,
        current: false,
        in_use: false,
    };
    const OTHER: Player = Player {
        spotify: false,
        ..SPOTIFY
    };

    fn playing(p: Player) -> Player {
        Player { playing: true, ..p }
    }
    fn in_use(p: Player) -> Player {
        Player { in_use: true, ..p }
    }
    fn current(p: Player) -> Player {
        Player { current: true, ..p }
    }

    #[test]
    fn an_idle_spotify_never_wins_over_a_player_that_is_playing() {
        assert_eq!(pick([SPOTIFY, playing(OTHER)]), Some(1));
        assert_eq!(pick([in_use(SPOTIFY), playing(OTHER)]), Some(1));
        assert_eq!(pick([current(SPOTIFY), playing(OTHER)]), Some(1));
    }

    #[test]
    fn spotify_comes_first_while_it_plays() {
        assert_eq!(pick([playing(in_use(OTHER)), playing(SPOTIFY)]), Some(1));
        assert_eq!(
            pick([playing(current(OTHER)), OTHER, playing(SPOTIFY)]),
            Some(2)
        );
    }

    #[test]
    fn the_player_in_use_keeps_the_widget() {
        // Another app starting doesn't steal it while it plays...
        assert_eq!(pick([playing(OTHER), playing(in_use(OTHER))]), Some(1));
        assert_eq!(
            pick([playing(current(OTHER)), playing(in_use(OTHER))]),
            Some(1)
        );
        // ...and pausing it doesn't hand the keys to an idle Spotify.
        assert_eq!(pick([SPOTIFY, in_use(OTHER)]), Some(1));
        // But one that is playing takes over from one that isn't.
        assert_eq!(pick([in_use(SPOTIFY), playing(OTHER)]), Some(1));
        assert_eq!(pick([in_use(OTHER), playing(OTHER)]), Some(1));
    }

    #[test]
    fn with_nothing_playing_or_in_use_the_system_then_spotify_then_order() {
        assert_eq!(pick([SPOTIFY, current(OTHER)]), Some(1));
        assert_eq!(pick([OTHER, SPOTIFY]), Some(1));
        assert_eq!(pick([OTHER, OTHER]), Some(0));
        assert_eq!(pick([playing(OTHER), playing(current(OTHER))]), Some(1));
        assert_eq!(pick([playing(OTHER), playing(OTHER)]), Some(0));
        assert_eq!(pick([]), None);
    }
}
