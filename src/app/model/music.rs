//! The music widget's state: the media source and the cover loader, both
//! alive only while the widget is placed (side or on the lava), and the
//! player keys (`A`).
//!
//! Nothing here waits on the player: [`Music::sync`] reads the source's
//! latest snapshot once a frame (a short lock) and commands are queued.

use std::time::{Duration, Instant};

use super::Model;
use super::library::ListKind;
use crate::dock::{self, DockWidget, Place};
use crate::media::art::{ArtLoader, ArtState};
use crate::media::{self, Capabilities, Command, MediaSource, Snapshot};
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
    /// Shuffle and repeat as the Web API reads them, when they're changed
    /// through it (the desktop app's own setters don't work: lava-75z.12).
    pub web_modes: Option<(bool, bool)>,
    connect: Connect,
    load_art: LoadArt,
}

impl Default for Music {
    fn default() -> Self {
        Self {
            source: None,
            art: None,
            snapshot: None,
            keys: false,
            web_modes: None,
            connect: Box::new(media::detect),
            load_art: Box::new(ArtLoader::start),
        }
    }
}

impl Music {
    /// Use `connect` for the player (tests: a `FakeSource`) and `load_art`
    /// for covers, from the next time the widget is placed.
    #[cfg(test)]
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
    /// for the cover when pictures can be shown.
    pub fn sync(&mut self, on: bool, images: bool) {
        if !on {
            self.source = None;
            self.art = None;
            self.snapshot = None;
            self.keys = false;
            return;
        }
        let source = self.source.get_or_insert_with(|| (self.connect)());
        let snapshot = source.snapshot();
        let url = snapshot
            .track
            .as_ref()
            .map(|t| t.artwork_url.as_str())
            .filter(|u| !u.is_empty());
        if let Some(url) = url.filter(|_| images) {
            self.art.get_or_insert_with(|| (self.load_art)()).want(url);
        }
        self.snapshot = Some(snapshot);
    }

    /// The cover of the playing track, as far as it has loaded.
    pub fn art(&self) -> ArtState {
        self.art.as_ref().map_or(ArtState::Loading, ArtLoader::get)
    }

    /// What the player can do: its own controls, plus shuffle / repeat
    /// through the Web API when that's on.
    pub fn capabilities(&self) -> Capabilities {
        let own = self.source_capabilities();
        let web = self.web_modes.is_some();
        Capabilities {
            shuffle: own.shuffle || web,
            repeat: own.repeat || web,
        }
    }

    fn source_capabilities(&self) -> Capabilities {
        self.source
            .as_ref()
            .map_or(Capabilities::NONE, |s| s.capabilities())
    }

    /// The desktop app plays `uri`, or why it can't.
    pub fn play_uri(&mut self, uri: &str, now: Instant) -> Result<(), String> {
        let snap = self.current().ok_or("music is off · a to show it")?;
        if !snap.status.is_available() {
            return Err(snap
                .unavailable_message()
                .unwrap_or_else(|| "connecting…".into()));
        }
        let command = Command::play_uri(uri).ok_or("can't play that")?;
        self.send(command, now);
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

    /// Whether anything needs the player: the music or the lyrics widget.
    pub fn media_on(&self) -> bool {
        self.music_on() || self.lyrics_on()
    }

    /// Connect or let go of the player as the widgets' places say, read its
    /// latest state (each frame, and after any key), and sync the lyrics
    /// to it.
    pub(super) fn sync_music(&mut self) {
        let images = self.music_on() && self.theme.shows_images();
        self.music.sync(self.media_on(), images);
        self.patch_modes();
        self.sync_lyrics();
    }

    /// The player key a mouse press at (`col`, `row`) stands for: a
    /// control or the progress bar of the music widget, wherever it's
    /// placed (the layout's own rects, so it matches what's drawn).
    pub(super) fn music_hit(&self, col: u16, row: u16) -> Option<PlayerKey> {
        let music = dock::by_name(dock::Music.name())?.0;
        let at = (col, row).into();
        self.layout
            .panel
            .iter()
            .chain(&self.layout.on_lava)
            .flat_map(|s| s.items.iter().map(move |p| (s.align, p)))
            .filter(|(_, p)| p.widget == music && p.rect.contains(at))
            .find_map(|(align, p)| dock::music_hit(self, p.form, p.rect, align, col, row))
    }

    /// `A`: the player keys on (they last until esc).
    pub(super) fn player_keys_on(&mut self) {
        if !self.music_on() {
            self.toast("music is off · a to show it");
            return;
        }
        self.music.keys = true;
        self.toast(match self.library.account() {
            super::library::Account::LoggedOut => "music keys · i log in · esc when done",
            _ => "music keys · esc when done",
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
        let web = self.music.web_modes.is_some();
        let player = snap.player_name().to_owned();
        let command = match key {
            PlayerKey::PlayPause => Command::PlayPause,
            PlayerKey::Next => Command::Next,
            PlayerKey::Previous => Command::Previous,
            PlayerKey::SeekBack => Command::Seek(snap.position_at(now).saturating_sub(SEEK_STEP)),
            PlayerKey::SeekForward => Command::Seek(snap.position_at(now) + SEEK_STEP),
            PlayerKey::VolumeUp | PlayerKey::VolumeDown => {
                let volume = if key == PlayerKey::VolumeUp {
                    snap.volume.saturating_add(VOLUME_STEP).min(100)
                } else {
                    snap.volume.saturating_sub(VOLUME_STEP)
                };
                self.toast(format!("volume {volume}"));
                Command::SetVolume(volume)
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
            PlayerKey::Shuffle | PlayerKey::Repeat if web => {
                self.web_mode(key == PlayerKey::Shuffle);
                return;
            }
            PlayerKey::Shuffle | PlayerKey::Repeat => {
                self.toast(if self.library.player.needs_login {
                    "log in to Spotify again for shuffle and repeat (i twice, then i)".to_owned()
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
