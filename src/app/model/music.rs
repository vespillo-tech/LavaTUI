//! The music widget's state: the media source and the cover loader, both
//! alive only while a widget that needs them is placed (music, lyrics,
//! cover), the player keys (`A`), and which cover the terminal's pixel
//! protocol should hold.
//!
//! Nothing here waits on the player: [`Music::sync`] reads the source's
//! latest snapshot once a frame (a short lock) and commands are queued.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::Model;
use super::library::{Account, ListKind};
use crate::dock::cover::{self, Drawn, Grain};
use crate::dock::{self, DockWidget, Place};
use crate::graphics::Protocol;
use crate::graphics::inline::Wish;
use crate::graphics::probe::Verdict;
use crate::media::art::{Art, ArtLoader, ArtState};
use crate::media::{self, Capabilities, Command, MediaSource, Snapshot};
use crate::theme::Rgb;
use crate::ui::keymap::PlayerKey;

/// `←` / `→` in the player keys.
pub const SEEK_STEP: Duration = Duration::from_secs(10);
/// `↑` / `↓` in the player keys.
pub const VOLUME_STEP: u8 = 5;

type Connect = Box<dyn Fn() -> Box<dyn MediaSource>>;
type LoadArt = Box<dyn Fn() -> ArtLoader>;

pub struct Music {
    source: Option<Box<dyn MediaSource>>,
    art: Option<ArtLoader>,
    /// The latest snapshot, read each frame; `None` while off.
    pub snapshot: Option<Snapshot>,
    /// The player keys are live (`A`): they take over the keyboard until
    /// esc.
    pub keys: bool,
    /// Shuffle and repeat the Web API can change right now (logged in,
    /// Premium, a device playing, the context allows it), where the
    /// desktop app's own setters don't work (lava-75z.12).
    pub web_caps: Capabilities,
    /// The source offered shuffle or repeat at the last sync: when that
    /// goes, the player was found to ignore them (lava-75z.21).
    own_modes: bool,
    connect: Connect,
    load_art: LoadArt,
    /// The last pixel-art picture made: of which cover, how many blocks.
    pixel_art: Option<PixelArt>,
}

/// A pixel-art picture ready to send: the cover, its blocks across and
/// down, the PNG (base64).
type PixelArt = (Arc<Art>, (u16, u16), Arc<String>);

impl Default for Music {
    fn default() -> Self {
        Self {
            source: None,
            art: None,
            snapshot: None,
            keys: false,
            web_caps: Capabilities::NONE,
            own_modes: false,
            connect: Box::new(media::detect),
            load_art: Box::new(ArtLoader::start),
            pixel_art: None,
        }
    }
}

impl Music {
    /// `art` as pixel art `grid` blocks across and down, ready to send:
    /// made once, then kept while the cover and grid stay.
    fn pixel_art(&mut self, art: &Arc<Art>, grid: (u16, u16)) -> Option<Arc<String>> {
        if let Some((made, at, png)) = &self.pixel_art
            && Arc::ptr_eq(made, art)
            && *at == grid
        {
            return Some(Arc::clone(png));
        }
        let png = art.pixel_art(grid)?;
        self.pixel_art = Some((Arc::clone(art), grid, Arc::clone(&png)));
        Some(png)
    }

    /// Use `connect` for the player (tests and `--demo`: a `FakeSource`) and `load_art`
    /// for covers, from the next time the widget is placed.
    pub fn connect_with(
        &mut self,
        connect: impl Fn() -> Box<dyn MediaSource> + 'static,
        load_art: impl Fn() -> ArtLoader + 'static,
    ) {
        self.connect = Box::new(connect);
        self.load_art = Box::new(load_art);
        self.source = None;
        self.art = None;
    }

    /// Once a frame: connect when `on` (the widget is placed), disconnect
    /// when not (which stops the polling), read the latest state, and ask
    /// for the cover when pictures can be shown (`hires`: the sharp copy
    /// for real pixels too). Whether the player has just been found to
    /// ignore shuffle and repeat.
    pub fn sync(&mut self, on: bool, images: bool, hires: bool) -> bool {
        if !on {
            self.source = None;
            self.art = None;
            self.snapshot = None;
            self.keys = false;
            self.own_modes = false;
            return false;
        }
        let source = self.source.get_or_insert_with(|| (self.connect)());
        let snapshot = source.snapshot();
        let caps = source.capabilities();
        let modes = caps.shuffle || caps.repeat;
        let lost = std::mem::replace(&mut self.own_modes, modes) && !modes;
        let url = snapshot
            .track
            .as_ref()
            .map(|t| t.artwork_url.as_str())
            .filter(|u| !u.is_empty());
        if let Some(url) = url.filter(|_| images) {
            let art = self.art.get_or_insert_with(|| (self.load_art)());
            art.set_hires(hires);
            art.want(url);
        }
        self.snapshot = Some(snapshot);
        lost
    }

    /// The cover of the playing track, as far as it has loaded.
    pub fn art(&self) -> ArtState {
        self.art.as_ref().map_or(ArtState::Loading, ArtLoader::get)
    }

    /// What the player can do: its own controls, plus shuffle / repeat
    /// through the Web API when that's on.
    pub fn capabilities(&self) -> Capabilities {
        let (own, web) = (self.source_capabilities(), self.web_caps);
        Capabilities {
            shuffle: own.shuffle || web.shuffle,
            repeat: own.repeat || web.repeat,
            ..own
        }
    }

    pub(super) fn source_capabilities(&self) -> Capabilities {
        self.source
            .as_ref()
            .map_or(Capabilities::NONE, |s| s.capabilities())
    }

    /// The desktop app plays `uri`, or why it can't.
    pub fn play_uri(&mut self, uri: &str, now: Instant) -> Result<(), String> {
        self.play(Command::play_uri(uri), now)
    }

    /// The desktop app plays `track` inside `context` (its playlist), so
    /// it carries on through the rest of it; or why it can't.
    pub fn play_in_context(
        &mut self,
        track: &str,
        context: &str,
        now: Instant,
    ) -> Result<(), String> {
        self.play(Command::play_in_context(track, context), now)
    }

    fn play(&mut self, command: Option<Command>, now: Instant) -> Result<(), String> {
        let snap = self.current().ok_or("music is off · a to show it")?;
        if !snap.status.is_available() {
            return Err(snap
                .unavailable_message()
                .unwrap_or_else(|| "connecting…".into()));
        }
        // Never queued (nor shown as playing) where it would do nothing.
        if !self.source_capabilities().uris {
            return Err(format!("{} can't be told what to play", snap.player_name()));
        }
        self.send(command.ok_or("can't play that")?, now);
        Ok(())
    }

    fn send(&mut self, command: Command, now: Instant) {
        if let Some(source) = &self.source {
            source.send(command.clone());
        }
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.apply(&command, now);
        }
    }

    /// The freshest state (not just this frame's), for a key press.
    fn current(&self) -> Option<Snapshot> {
        self.source.as_ref().map(|s| s.snapshot())
    }
}

impl Model {
    /// Whether the music widget is placed (its source is alive).
    pub fn music_on(&self) -> bool {
        self.settings.dock.place(&dock::Music) != Place::Off
    }

    /// Whether the cover widget is placed.
    pub fn cover_on(&self) -> bool {
        self.settings.dock.place(&dock::Cover) != Place::Off
    }

    /// Whether anything needs the player: the music, lyrics or cover
    /// widget, or the Spotify setup (it says how the app is doing).
    pub fn media_on(&self) -> bool {
        self.music_on() || self.lyrics_on() || self.cover_on() || self.spotify_setup_open()
    }

    /// How covers are drawn here and now (`art.detail` resolved).
    pub fn pictures(&self) -> Drawn {
        cover::resolve(self.settings.art.detail, self.caps, self.theme.depth())
    }

    /// Whether the music card shows its own small cover.
    pub fn inline_cover(&self) -> bool {
        self.settings.art.inline && !self.cover_on() && self.pictures() != Drawn::None
    }

    /// Connect or let go of the player as the widgets' places say, read its
    /// latest state (each frame, and after any key), and sync the lyrics
    /// to it.
    pub(super) fn sync_music(&mut self) {
        let pictures = self.pictures();
        let images = (self.music_on() && self.inline_cover()) || self.cover_on();
        let images = images && pictures != Drawn::None;
        let ignored = self.music.sync(
            self.media_on(),
            images,
            matches!(pictures, Drawn::Pixels(..)),
        );
        self.patch_modes();
        if ignored {
            self.modes_ignored();
        }
        self.sync_lyrics();
    }

    /// String replies from the terminal (`app::replies`): the answer to
    /// the picture probe, if one is among them. Whether it settled it
    /// (a redraw is due).
    pub fn terminal_replies(&mut self, replies: &[String]) -> bool {
        let mut settled = false;
        for reply in replies {
            if let Some(verdict) = self.probe.as_ref().and_then(|p| p.reply(reply)) {
                self.settle_probe(verdict);
                settled = true;
            }
        }
        settled
    }

    /// The probe's answer: pixels from now on, or (if covers were going
    /// to use them) a toast that they're drawn in text cells instead.
    pub(super) fn settle_probe(&mut self, verdict: Verdict) {
        let Some(probe) = self.probe.take() else {
            return;
        };
        if verdict == Verdict::Yes {
            self.caps.pixels = Some(probe.protocol);
            return;
        }
        let covers = self.cover_on() || (self.music_on() && self.settings.art.inline);
        // Pixel art looks the same in text cells; a sharp cover doesn't.
        let sharp = self.settings.art.detail.grain() == Grain::Sharp;
        if covers && sharp && matches!(self.pictures(), Drawn::Text(_)) {
            self.toast("no photos in this terminal · covers drawn in text");
        }
    }

    /// After each layout: the picture the terminal should hold (the cover
    /// at the size it's laid out at, in pixels mode), or none.
    pub(super) fn sync_pictures(&mut self) {
        let want = (|| {
            let Drawn::Pixels(protocol, grain) = self.pictures() else {
                return None;
            };
            let ArtState::Ready(art) = self.music.art() else {
                return None;
            };
            let track = self.music.snapshot.as_ref()?.track.as_ref()?;
            let r = dock::cover_at(&self.layout)?;
            let key = cover::picture_key(&track.artwork_url, r, grain);
            let png = match key.grid {
                None => art.hires.clone()?,
                Some(grid) => self.music.pixel_art(&art, grid)?,
            };
            let Rgb(red, green, blue) = art.mean();
            Some(Wish {
                protocol,
                key,
                at: r,
                png,
                cell: self.cell_px,
                bg: [red, green, blue],
            })
        })();
        let (kitty, inline) = match want {
            Some(w) if w.protocol == Protocol::Kitty => (Some(w), None),
            w => (None, w),
        };
        self.kitty
            .want(kitty.as_ref().map(|w| (w.key.clone(), &w.png)));
        self.inline.want(inline, self.layout.area);
    }

    /// The player key a mouse press at (`col`, `row`) stands for: a
    /// control or the progress bar of the music widget, or the cover
    /// (play / pause), wherever they're placed (the layout's own rects, so
    /// it matches what's drawn).
    pub(super) fn music_hit(&self, col: u16, row: u16) -> Option<PlayerKey> {
        let music = dock::by_name(dock::Music.name())?.0;
        let cover = dock::by_name(dock::Cover.name())?.0;
        let at = (col, row).into();
        let mut placed = self
            .layout
            .panel
            .iter()
            .chain(&self.layout.on_lava)
            .flat_map(|s| s.items.iter().map(move |p| (s.align, p)))
            .filter(|(_, p)| p.rect.contains(at));
        placed.find_map(|(align, p)| {
            if p.widget == music {
                dock::music_hit(self, p.form, p.rect, align, col, row)
            } else if p.widget == cover {
                let r = cover::picture_rect(p.form, p.rect, align)?;
                let snap = self.music.snapshot.as_ref()?;
                (r.contains(at) && snap.track.is_some() && snap.status.is_available())
                    .then_some(PlayerKey::PlayPause)
            } else {
                None
            }
        })
    }

    /// `O`: the next cover detail; the toast says what it comes to here.
    pub(super) fn next_cover_detail(&mut self, now: Instant) {
        let art = &mut self.settings.art;
        art.detail = art.detail.next();
        let detail = art.detail;
        self.changed(now);
        self.sync_music();
        let drawn = self.pictures().label();
        self.toast(if drawn == detail.label() {
            format!("cover quality · {drawn}")
        } else {
            format!("cover quality · {} · {drawn}", detail.label())
        });
    }

    /// The player took a shuffle / repeat change and didn't make it
    /// (Spotify over MPRIS, lava-75z.21): the change it showed is undone
    /// by now, so say so, and what does work.
    fn modes_ignored(&mut self) {
        let Some(snap) = &self.music.snapshot else {
            return;
        };
        let player = snap.player_name().to_owned();
        let web = self.music.web_caps;
        let next = if web.shuffle || web.repeat {
            "press again to use your Spotify account"
        } else if snap.is_spotify() && self.library.account() == Account::LoggedOut {
            "log in (i) to shuffle and repeat"
        } else {
            "it can't shuffle or repeat from here"
        };
        self.toast(format!("{player} ignored that · {next}"));
    }

    /// `A`: the player keys on (they last until esc).
    pub(super) fn player_keys_on(&mut self) {
        if !self.music_on() {
            self.toast("music is off · a to show it");
            return;
        }
        self.music.keys = true;
        self.toast(match self.library.account() {
            Account::LoggedOut => "music controls · i log in · Esc back",
            _ => "music controls · Esc back",
        });
    }

    /// One player key: the command goes to the player at once, and shows on
    /// the next frame (optimistically, before the player confirms).
    pub(super) fn player_key(&mut self, key: PlayerKey, now: Instant) {
        // The library keys don't need the desktop app.
        match key {
            PlayerKey::Like => return self.like_key(),
            PlayerKey::AddToPlaylist => return self.open_library(ListKind::AddTo),
            PlayerKey::Playlists => return self.open_library(ListKind::Playlists),
            PlayerKey::Account => return self.account_key(now),
            _ => {}
        }
        let Some(snap) = self.music.current() else {
            return;
        };
        if !snap.status.is_available() {
            let message = snap
                .unavailable_message()
                .unwrap_or_else(|| "connecting…".into());
            self.toast(message);
            return;
        }
        let caps = self.music.source_capabilities();
        let web = self.music.web_caps;
        let player = snap.player_name().to_owned();
        let command = match key {
            PlayerKey::PlayPause => Command::PlayPause,
            PlayerKey::Next => Command::Next,
            PlayerKey::Previous => Command::Previous,
            PlayerKey::SeekBack => Command::Seek(snap.position_at(now).saturating_sub(SEEK_STEP)),
            PlayerKey::SeekForward => Command::Seek(snap.position_at(now) + SEEK_STEP),
            PlayerKey::VolumeUp | PlayerKey::VolumeDown if !caps.volume => {
                self.toast(format!("{player} has no volume control here"));
                return;
            }
            PlayerKey::VolumeUp | PlayerKey::VolumeDown => {
                let volume = if key == PlayerKey::VolumeUp {
                    snap.volume.saturating_add(VOLUME_STEP).min(100)
                } else {
                    snap.volume.saturating_sub(VOLUME_STEP)
                };
                self.toast(format!("volume {volume}"));
                Command::SetVolume(volume)
            }
            // The modes shown are the account's (patch_modes): change them
            // there, not in a player that may ignore it (lava-75z.21).
            PlayerKey::Shuffle if web.shuffle => {
                self.web_mode(true);
                return;
            }
            PlayerKey::Repeat if web.repeat => {
                self.web_mode(false);
                return;
            }
            PlayerKey::Shuffle if caps.shuffle => {
                self.toast(if snap.shuffle {
                    "shuffle off"
                } else {
                    "shuffle on"
                });
                Command::SetShuffle(!snap.shuffle)
            }
            PlayerKey::Repeat if caps.repeat => {
                self.toast(if snap.repeat {
                    "repeat off"
                } else {
                    "repeat on"
                });
                Command::SetRepeat(!snap.repeat)
            }
            // Whether the account can do it is in the saved login, unread
            // so far (macOS may ask first): read it, then try again.
            PlayerKey::Shuffle | PlayerKey::Repeat
                if snap.is_spotify() && self.library.locked() =>
            {
                self.unlock_library(Some(key));
                return;
            }
            PlayerKey::Shuffle | PlayerKey::Repeat if self.web_modes().is_some() => {
                self.toast("Spotify won't change that for what's playing");
                return;
            }
            PlayerKey::Shuffle | PlayerKey::Repeat => {
                let logged_out = self.library.account() == Account::LoggedOut;
                self.toast(if self.library.player.needs_login {
                    "log in to Spotify again for shuffle and repeat (i twice, then i)".to_owned()
                } else if snap.is_spotify() && logged_out {
                    format!("{player} can't shuffle or repeat from here · log in (i) to")
                } else {
                    format!("{player} can't shuffle or repeat from here")
                });
                return;
            }
            PlayerKey::SeekTo(permille) => match snap.track.as_ref().map(|t| t.duration) {
                Some(d) if !d.is_zero() => {
                    Command::Seek(d.mul_f64(f64::from(permille.min(1000)) / 1000.0))
                }
                _ => return,
            },
            PlayerKey::Like
            | PlayerKey::AddToPlaylist
            | PlayerKey::Playlists
            | PlayerKey::Account => return,
        };
        self.music.send(command, now);
    }
}
