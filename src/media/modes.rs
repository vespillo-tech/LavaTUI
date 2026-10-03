//! Whether a player does what shuffle / repeat are set to, learned from
//! the reads after a change (MPRIS players, Apple Music).

use std::time::{Duration, Instant};

use super::{Command, Snapshot};

/// How long a player may take to show a shuffle / repeat change before
/// it's taken as ignored (reads every second or faster while it plays).
pub const MODES_GRACE: Duration = Duration::from_millis(1500);

/// Whether the player does what `Shuffle` / `LoopStatus` are set to,
/// learned from the reads after a change (lava-75z.21). Spotify on Linux
/// has long accepted both and changed nothing; other players (and maybe
/// newer Spotify clients) honour them. Until a change is seen not to
/// stick, they're assumed to work; once one doesn't within
/// [`MODES_GRACE`], they're [`Self::ignored`] for that player, so its
/// keys stop claiming a change (and Spotify's go through the Web API
/// when logged in, as on macOS).
#[derive(Debug, Default)]
pub struct ModesCheck {
    /// What the last change asked for (shuffle, repeat; `None`: not
    /// changed) and when.
    expect: Option<(Option<bool>, Option<bool>, Instant)>,
    ignored: bool,
}

impl ModesCheck {
    /// `command` went through (the player accepted the call) at `now`.
    pub fn sent(&mut self, command: &Command, now: Instant) {
        let (mut shuffle, mut repeat) = self.expect.map_or((None, None), |(s, r, _)| (s, r));
        match command {
            Command::SetShuffle(on) => shuffle = Some(*on),
            Command::SetRepeat(on) => repeat = Some(*on),
            _ => return,
        }
        self.expect = Some((shuffle, repeat, now));
    }

    /// A read of the player (`snap`, available) at `now`: the change shows,
    /// or hasn't in time.
    pub fn read(&mut self, snap: &Snapshot, now: Instant) {
        let Some((shuffle, repeat, at)) = self.expect else {
            return;
        };
        let shows = shuffle.is_none_or(|on| on == snap.shuffle)
            && repeat.is_none_or(|on| on == snap.repeat);
        if shows {
            self.expect = None;
        } else if now.saturating_duration_since(at) >= MODES_GRACE {
            self.expect = None;
            self.ignored = true;
        }
    }

    /// The player was seen to ignore a change.
    pub fn ignored(&self) -> bool {
        self.ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Status;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn modes_a_player_ignores_are_learned_from_the_reads_after() {
        let t0 = Instant::now();
        let read = |shuffle, repeat| Snapshot {
            shuffle,
            repeat,
            ..Snapshot::new(Status::Playing, t0)
        };
        // Honoured: the read shows it, even a moment late.
        let mut check = ModesCheck::default();
        check.sent(&Command::SetShuffle(true), t0);
        check.read(&read(false, false), t0);
        check.read(&read(true, false), t0 + S);
        check.read(&read(false, false), t0 + S * 5);
        assert!(!check.ignored(), "a later change by hand is not ignoring");

        // Ignored: still not there once the grace is up.
        let mut check = ModesCheck::default();
        check.sent(&Command::SetShuffle(true), t0);
        check.read(&read(false, false), t0 + MODES_GRACE / 2);
        assert!(!check.ignored(), "not yet");
        check.read(&read(false, false), t0 + MODES_GRACE);
        assert!(check.ignored());

        // Both asked for: both must show.
        let mut check = ModesCheck::default();
        check.sent(&Command::SetShuffle(true), t0);
        check.sent(&Command::SetRepeat(true), t0);
        check.read(&read(true, false), t0 + S * 2);
        assert!(check.ignored());

        // Other commands say nothing about it.
        let mut check = ModesCheck::default();
        check.sent(&Command::SetVolume(10), t0);
        check.read(&read(false, false), t0 + S * 9);
        assert!(!check.ignored());
    }
}
