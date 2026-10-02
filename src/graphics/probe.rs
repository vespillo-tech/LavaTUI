//! Checking that the terminal really shows the pictures its environment
//! promises, without ever waiting on it.
//!
//! The environment can lie: a terminal embedded in another app may
//! inherit `TERM_PROGRAM=ghostty` and not speak kitty graphics at all (its
//! placeholders then show as `?` boxes). So at start the app writes
//! [`Probe::query`] once and carries on; until an answer settles it, the
//! cover is drawn in text cells. Replies arrive as input, are taken out of
//! the key stream by `app::replies` and handed here as strings
//! ([`Probe::reply`]); a deadline ([`Probe::expired`]) settles it if they
//! never come.
//!
//! * Kitty: a graphics query (`a=q`, a 1×1 image that's never stored).
//!   `OK` → yes; an error, or the fence arriving first, or nothing → no.
//! * iTerm2 / sixel: there's no query crossterm lets through (DA1, whose
//!   `4` means sixel, it swallows), so the terminal is asked its name
//!   (XTVERSION). A name that isn't one known to speak the protocol → no;
//!   no name (many don't answer) → the environment is believed.
//!
//! Every query is followed by a fence: OSC 10 (the foreground colour),
//! which nearly every terminal answers, and answers in order: once it's
//! back, anything before it that was going to be answered has been.

use std::time::{Duration, Instant};

use super::Protocol;

/// How long to wait for an answer before settling without one.
pub const WAIT: Duration = Duration::from_millis(1500);

/// The kitty query's image id (never stored: `a=q`).
const QUERY_ID: u32 = 31;

/// Terminals (as XTVERSION names them, lowercase) known to speak each
/// protocol that has no query of its own.
const ITERM: &[&str] = &["iterm2", "wezterm", "mintty", "rio"];
const SIXEL: &[&str] = &[
    "foot", "mlterm", "konsole", "contour", "xterm", "wezterm", "mintty", "iterm2", "rio",
];

/// What the probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Yes,
    No,
}

/// A question in flight.
#[derive(Debug, Clone)]
pub struct Probe {
    pub protocol: Protocol,
    deadline: Instant,
}

impl Probe {
    /// A probe for `protocol`, asked at `now`.
    pub fn new(protocol: Protocol, now: Instant) -> Self {
        Self {
            protocol,
            deadline: now + WAIT,
        }
    }

    /// The bytes that ask: the question, then the fence.
    pub fn query(&self) -> Vec<u8> {
        let mut out = match self.protocol {
            Protocol::Kitty => {
                format!("\x1b_Gi={QUERY_ID},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\").into_bytes()
            }
            Protocol::Iterm | Protocol::Sixel => b"\x1b[>0q".to_vec(),
        };
        out.extend_from_slice(b"\x1b]10;?\x1b\\");
        out
    }

    /// One string reply, as `app::replies` collects it: the introducer
    /// (`_` APC, `P` DCS, `]` OSC) and the body. A verdict, if it settles
    /// it.
    pub fn reply(&self, reply: &str) -> Option<Verdict> {
        let fence = reply.starts_with("]10;");
        match self.protocol {
            Protocol::Kitty => {
                if let Some(body) = reply.strip_prefix("_G") {
                    let (controls, message) = body.split_once(';').unwrap_or((body, ""));
                    let ours = controls.split(',').any(|kv| kv == format!("i={QUERY_ID}"));
                    return ours.then_some(if message == "OK" {
                        Verdict::Yes
                    } else {
                        Verdict::No
                    });
                }
                fence.then_some(Verdict::No)
            }
            Protocol::Iterm | Protocol::Sixel => {
                if let Some(name) = reply.strip_prefix("P>|") {
                    let known = match self.protocol {
                        Protocol::Iterm => ITERM,
                        _ => SIXEL,
                    };
                    let name = name.to_lowercase();
                    let yes = known.iter().any(|k| name.starts_with(k));
                    return Some(if yes { Verdict::Yes } else { Verdict::No });
                }
                fence.then_some(Verdict::Yes)
            }
        }
    }

    /// The verdict once nothing came by the deadline: kitty no (it always
    /// answers a query it understands), the others believed.
    pub fn expired(&self, now: Instant) -> Option<Verdict> {
        (now >= self.deadline).then_some(match self.protocol {
            Protocol::Kitty => Verdict::No,
            Protocol::Iterm | Protocol::Sixel => Verdict::Yes,
        })
    }

    /// When it expires (frames mustn't sleep past it).
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(p: Protocol) -> Probe {
        Probe::new(p, Instant::now())
    }

    #[test]
    fn the_kitty_query_asks_then_fences() {
        let q = String::from_utf8(probe(Protocol::Kitty).query()).unwrap();
        assert_eq!(
            q,
            "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b]10;?\x1b\\"
        );
        let q = String::from_utf8(probe(Protocol::Sixel).query()).unwrap();
        assert_eq!(q, "\x1b[>0q\x1b]10;?\x1b\\");
    }

    #[test]
    fn kitty_is_yes_only_on_ok() {
        let p = probe(Protocol::Kitty);
        assert_eq!(p.reply("_Gi=31;OK"), Some(Verdict::Yes));
        assert_eq!(p.reply("_Gi=31;EINVAL:bad"), Some(Verdict::No));
        // The fence first: the query went unanswered.
        assert_eq!(p.reply("]10;rgb:ffff/ffff/ffff"), Some(Verdict::No));
        // Someone else's image reply, a name: not ours to judge.
        assert_eq!(p.reply("_Gi=7;OK"), None);
        assert_eq!(p.reply("P>|ghostty 1.2.0"), None);
    }

    #[test]
    fn iterm_and_sixel_check_the_name_when_there_is_one() {
        let iterm = probe(Protocol::Iterm);
        assert_eq!(iterm.reply("P>|iTerm2 3.5.4"), Some(Verdict::Yes));
        assert_eq!(
            iterm.reply("P>|WezTerm 20240203-110809"),
            Some(Verdict::Yes)
        );
        assert_eq!(iterm.reply("P>|ghostty 1.1.3"), Some(Verdict::No));
        assert_eq!(iterm.reply("P>|tmux 3.4"), Some(Verdict::No));
        assert_eq!(iterm.reply("]10;rgb:0/0/0"), Some(Verdict::Yes), "no name");
        let sixel = probe(Protocol::Sixel);
        assert_eq!(sixel.reply("P>|foot(1.18.1)"), Some(Verdict::Yes));
        assert_eq!(sixel.reply("P>|XTerm(390)"), Some(Verdict::Yes));
        assert_eq!(sixel.reply("P>|kitty(0.36.0)"), Some(Verdict::No));
        assert_eq!(sixel.reply("]11;rgb:0/0/0"), None, "not the fence");
    }

    #[test]
    fn silence_settles_at_the_deadline() {
        let t0 = Instant::now();
        let kitty = Probe::new(Protocol::Kitty, t0);
        assert_eq!(kitty.expired(t0), None);
        assert_eq!(kitty.expired(t0 + WAIT), Some(Verdict::No));
        let sixel = Probe::new(Protocol::Sixel, t0);
        assert_eq!(sixel.expired(t0 + WAIT), Some(Verdict::Yes));
        assert_eq!(sixel.deadline(), t0 + WAIT);
    }
}
