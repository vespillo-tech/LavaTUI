//! The music widget's state: the media source and the cover loader, both
//! alive only while the widget is placed (side or on the lava), and the
//! player keys (`A`).
//!
//! Nothing here waits on the player: [`Music::sync`] reads the source's
//! latest snapshot once a frame (a short lock) and commands are queued.

use std::time::{Duration, Instant};

use super::Model;
use crate::dock::{self, Place};
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

    pub fn capabilities(&self) -> Capabilities {
        self.source
            .as_ref()
            .map_or(Capabilities::NONE, |s| s.capabilities())
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
        self.sync_lyrics();
    }

    /// `A`: the player keys on (they last until esc).
    pub(super) fn player_keys_on(&mut self) {
        if !self.music_on() {
            self.toast("music is off · a to show it");
            return;
        }
        self.music.keys = true;
        self.toast("music keys · esc when done");
    }

    /// One player key: the command goes to the player at once, and shows on
    /// the next frame (optimistically, before the player confirms).
    pub(super) fn player_key(&mut self, key: PlayerKey, now: Instant) {
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
        let caps = self.music.capabilities();
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
            PlayerKey::Shuffle | PlayerKey::Repeat => {
                self.toast(format!("{player} can't shuffle or repeat from here"));
                return;
            }
        };
        self.music.send(command, now);
    }
}
