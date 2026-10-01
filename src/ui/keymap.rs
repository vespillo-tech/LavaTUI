//! The keymap (docs/design.md §6): one table, [`KEYMAP`], drives both
//! dispatch and the help overlay, so they can't drift.
//!
//! Global keys only fire when no overlay is open (except `ctrl-c`, which
//! always quits). Overlays have their own small fixed key sets. `esc` never
//! quits: it closes overlays and is a no-op otherwise.
//!
//! The player keys (the [`Section::Music`] rows) are a mode of their own:
//! `A` turns them on and they take the keyboard (like an overlay, but with
//! no sheet) until `esc`, `q` or `A` again. That keeps one global key for
//! the whole player instead of nine, and lets them reuse the obvious
//! letters (`␣`, `n`, `p`, arrows) the lamp and pomodoro already own.

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    /// esc: close the overlay (a picker reverts).
    Close,
    ToggleMinimal,
    ToggleStatusBar,
    NextStyle,
    StylePicker,
    NextFace,
    FacePicker,
    NextPalette,
    PalettePicker,
    /// Move the dock widget of this name on: side → lava → off → side.
    Place(&'static str),
    /// Move the focused widget on the lava to the next spot.
    NextAnchor,
    /// Focus the next widget on the lava (for `l`).
    NextLavaWidget,
    /// `A`: the player keys on (in them: off again).
    PlayerKeys,
    /// One of the player keys (only while they're on).
    Player(PlayerKey),
    ToggleHour24,
    PomodoroToggle,
    PomodoroSkip,
    PomodoroReset,
    HeatDown,
    HeatUp,
    Slower,
    Faster,
    ResetHeatSpeed,
    Freeze,
    Reseed,
    DebugHud,
    Redraw,
    // In overlays.
    Up,
    Down,
    /// Keep the picker's current item and close it.
    Keep,
    /// Jump the picker cursor to item `n` (0-based).
    Jump(u8),
    /// A page down (`true`) or up in a list.
    Page(bool),
    /// To the end (`true`) or the start of a list.
    Edge(bool),
    /// Up a level in the library (a playlist's tracks → the playlists);
    /// closes it at the top.
    Back,
    /// Play the whole playlist under the cursor (or the open one).
    PlayAll,
    // Not keys.
    /// The terminal was resized: relayout and redraw now.
    Resize,
    Focus(bool),
    /// Mouse click/drag at a screen cell: a heat pulse if it's on the wax.
    Poke {
        col: u16,
        row: u16,
    },
    /// Mouse click in a picker: preview the item there (twice: keep it).
    Click {
        col: u16,
        row: u16,
    },
    /// Mouse press with nothing open: a music widget button or the
    /// progress bar if it's on one, else a heat pulse on the wax.
    Press {
        col: u16,
        row: u16,
    },
}

/// What a player key does (`app/model/music.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerKey {
    PlayPause,
    Next,
    Previous,
    SeekBack,
    SeekForward,
    VolumeDown,
    VolumeUp,
    Shuffle,
    Repeat,
    /// Like / unlike the playing track (Web API).
    Like,
    /// The add-to-playlist picker.
    AddToPlaylist,
    /// The playlist browser.
    Playlists,
    /// Log in to / out of Spotify (Web API).
    Account,
    /// Jump to this far through the track, in ‰ (a click on the bar).
    SeekTo(u16),
}

/// A key as written in the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Space,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Lamp,
    Clock,
    /// Where the dock widgets go.
    Widgets,
    App,
    /// The player keys: live only after `A` ([`InputMode::Player`]).
    Music,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Lamp => "lamp",
            Section::Clock => "clock & pomodoro",
            Section::Widgets => "widgets",
            Section::App => "app",
            Section::Music => "music · after A",
        }
    }
}

/// One help line and the bindings it documents.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    pub section: Section,
    /// Keys as shown in help (`[ ]`, `r r`, `␣`).
    pub keys: &'static str,
    pub label: &'static str,
    pub binds: &'static [(Key, Action)],
}

const fn row(
    section: Section,
    keys: &'static str,
    label: &'static str,
    binds: &'static [(Key, Action)],
) -> Row {
    Row {
        section,
        keys,
        label,
        binds,
    }
}

use Action as A;
use Key::{Char as K, Ctrl};
use PlayerKey as P;
use Section::{App, Clock, Lamp, Music, Widgets};

pub static KEYMAP: &[Row] = &[
    row(Lamp, "s", "next style", &[(K('s'), A::NextStyle)]),
    row(Lamp, "S", "style picker", &[(K('S'), A::StylePicker)]),
    row(Lamp, "p", "next palette", &[(K('p'), A::NextPalette)]),
    row(Lamp, "P", "palette picker", &[(K('P'), A::PalettePicker)]),
    row(
        Lamp,
        "[ ]",
        "heat − +",
        &[(K('['), A::HeatDown), (K(']'), A::HeatUp)],
    ),
    row(
        Lamp,
        "- +",
        "speed",
        &[
            (K('-'), A::Slower),
            (K('+'), A::Faster),
            (K('='), A::Faster),
        ],
    ),
    row(Lamp, "z", "freeze", &[(K('z'), A::Freeze)]),
    row(
        Lamp,
        "0",
        "reset heat & speed",
        &[(K('0'), A::ResetHeatSpeed)],
    ),
    row(Lamp, "R", "reseed wax", &[(K('R'), A::Reseed)]),
    row(Clock, "c", "next face", &[(K('c'), A::NextFace)]),
    row(Clock, "C", "face picker", &[(K('C'), A::FacePicker)]),
    row(Clock, "T", "12h / 24h", &[(K('T'), A::ToggleHour24)]),
    row(
        Clock,
        "␣",
        "start / pause",
        &[(Key::Space, A::PomodoroToggle)],
    ),
    row(Clock, "n", "skip phase", &[(K('n'), A::PomodoroSkip)]),
    row(
        Clock,
        "r r",
        "reset pomodoro",
        &[(K('r'), A::PomodoroReset)],
    ),
    // One row per dock widget (`Action::Place` names it), plus the anchor.
    row(
        Widgets,
        "t",
        "clock side/lava/off",
        &[(K('t'), A::Place("clock"))],
    ),
    row(
        Widgets,
        "f",
        "pomodoro side/lava/off",
        &[(K('f'), A::Place("pomodoro"))],
    ),
    row(
        Widgets,
        "a",
        "music side/lava/off",
        &[(K('a'), A::Place("music"))],
    ),
    row(Widgets, "A", "music keys", &[(K('A'), A::PlayerKeys)]),
    row(
        Widgets,
        "y",
        "lyrics · lrclib.net",
        &[(K('y'), A::Place("lyrics"))],
    ),
    row(
        Widgets,
        "l L",
        "move, pick lava widget",
        &[(K('l'), A::NextAnchor), (K('L'), A::NextLavaWidget)],
    ),
    // m ? q first: the small full-screen help leads with them (§4.3).
    row(App, "m", "minimal", &[(K('m'), A::ToggleMinimal)]),
    row(App, "?", "this help", &[(K('?'), A::Help)]),
    row(
        App,
        "q",
        "quit · ctrl-c",
        &[(K('q'), A::Quit), (Ctrl('c'), A::Quit)],
    ),
    row(
        App,
        "b d",
        "status bar · debug hud",
        &[(K('b'), A::ToggleStatusBar), (K('d'), A::DebugHud)],
    ),
    row(App, "ctrl-l", "redraw", &[(Ctrl('l'), A::Redraw)]),
    // The player keys, after `A` (their own mode: they may reuse keys).
    row(
        Music,
        "␣ n p",
        "play · next · previous",
        &[
            (Key::Space, A::Player(P::PlayPause)),
            (K('n'), A::Player(P::Next)),
            (K('p'), A::Player(P::Previous)),
        ],
    ),
    row(
        Music,
        "←→ ↑↓",
        "seek · volume",
        &[
            (Key::Left, A::Player(P::SeekBack)),
            (Key::Right, A::Player(P::SeekForward)),
            (K('h'), A::Player(P::SeekBack)),
            (K('l'), A::Player(P::SeekForward)),
            (Key::Up, A::Player(P::VolumeUp)),
            (Key::Down, A::Player(P::VolumeDown)),
            (K('k'), A::Player(P::VolumeUp)),
            (K('j'), A::Player(P::VolumeDown)),
            (K('+'), A::Player(P::VolumeUp)),
            (K('='), A::Player(P::VolumeUp)),
            (K('-'), A::Player(P::VolumeDown)),
        ],
    ),
    row(
        Music,
        "x r",
        "shuffle · repeat",
        &[
            (K('x'), A::Player(P::Shuffle)),
            (K('r'), A::Player(P::Repeat)),
        ],
    ),
    // The Spotify library (Web API, `docs/spotify.md`).
    row(
        Music,
        "s a",
        "like · add to playlist",
        &[
            (K('s'), A::Player(P::Like)),
            (K('a'), A::Player(P::AddToPlaylist)),
        ],
    ),
    row(
        Music,
        "b i",
        "playlists · log in/out",
        &[
            (K('b'), A::Player(P::Playlists)),
            (K('i'), A::Player(P::Account)),
        ],
    ),
];

/// Which key set is live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Help,
    /// A picker; `opener` keeps + closes it, `inline` adds h/l ←/→.
    Picker {
        opener: Action,
        inline: bool,
    },
    /// The player keys (`A`); `esc`, `q` and `A` leave them.
    Player,
    /// The playlist browser / add-to-playlist picker; `inline` adds
    /// h/l ←/→ as move (else they go back / open).
    Library {
        inline: bool,
    },
}

pub fn action_for(event: &Event, mode: InputMode) -> Option<Action> {
    match event {
        // A held r must never reset the pomodoro (where the terminal tells
        // repeats apart; the model guards the rest).
        Event::Key(key) if key.kind != KeyEventKind::Release => key_action(key, mode)
            .filter(|&a| !(key.kind == KeyEventKind::Repeat && a == Action::PomodoroReset)),
        Event::Mouse(mouse) => mouse_action(mouse, mode),
        Event::Resize(..) => Some(Action::Resize),
        Event::FocusGained => Some(Action::Focus(true)),
        Event::FocusLost => Some(Action::Focus(false)),
        _ => None,
    }
}

/// The key a crossterm event stands for, if it's one we can bind.
fn key_of(key: &KeyEvent) -> Option<Key> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Char(' ') if !ctrl => Some(Key::Space),
        KeyCode::Char(c) if ctrl => Some(Key::Ctrl(c.to_ascii_lowercase())),
        // Terminals with the kitty protocol report shift separately.
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(Key::Char(c.to_ascii_uppercase()))
        }
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        _ => None,
    }
}

fn key_action(event: &KeyEvent, mode: InputMode) -> Option<Action> {
    let key = key_of(event);
    if key == Some(Ctrl('c')) {
        return Some(Action::Quit);
    }
    let code = event.code;
    match mode {
        InputMode::Normal => {
            if code == KeyCode::Esc {
                return None;
            }
            binding(Section::is_global, key?)
        }
        InputMode::Player => match (code, key) {
            (KeyCode::Esc, _) | (_, Some(K('q'))) => Some(Action::Close),
            (_, Some(k)) => match binding(|s| s == Music, k) {
                Some(a) => Some(a),
                None => match binding(Section::is_global, k)? {
                    Action::PlayerKeys => Some(Action::Close),
                    Action::Help => Some(Action::Help),
                    _ => None,
                },
            },
            _ => None,
        },
        InputMode::Help => match (code, key) {
            (KeyCode::Esc, _) | (_, Some(K('?') | K('q'))) => Some(Action::Close),
            (KeyCode::Up, _) | (_, Some(K('k'))) => Some(Action::Up),
            (KeyCode::Down, _) | (_, Some(K('j'))) => Some(Action::Down),
            _ => None,
        },
        InputMode::Library { inline } => match (code, key) {
            (KeyCode::Esc, _) => Some(Action::Back),
            (_, Some(K('q'))) => Some(Action::Close),
            (KeyCode::Enter, _) | (_, Some(Key::Space)) => Some(Action::Keep),
            (KeyCode::Up, _) | (_, Some(K('k'))) => Some(Action::Up),
            (KeyCode::Down, _) | (_, Some(K('j'))) => Some(Action::Down),
            (KeyCode::Left, _) | (_, Some(K('h'))) if inline => Some(Action::Up),
            (KeyCode::Right, _) | (_, Some(K('l'))) if inline => Some(Action::Down),
            (KeyCode::Left, _) | (_, Some(K('h'))) => Some(Action::Back),
            (KeyCode::Right, _) | (_, Some(K('l'))) => Some(Action::Keep),
            (KeyCode::PageUp, _) => Some(Action::Page(false)),
            (KeyCode::PageDown, _) => Some(Action::Page(true)),
            (KeyCode::Home, _) | (_, Some(K('g'))) => Some(Action::Edge(false)),
            (KeyCode::End, _) | (_, Some(K('G'))) => Some(Action::Edge(true)),
            (_, Some(K('p'))) => Some(Action::PlayAll),
            _ => None,
        },
        InputMode::Picker { opener, inline } => match (code, key) {
            (KeyCode::Esc, _) | (_, Some(K('q'))) => Some(Action::Close),
            (KeyCode::Enter, _) | (_, Some(Key::Space)) => Some(Action::Keep),
            (KeyCode::Up, _) | (_, Some(K('k'))) => Some(Action::Up),
            (KeyCode::Down, _) | (_, Some(K('j'))) => Some(Action::Down),
            (KeyCode::Left, _) | (_, Some(K('h'))) if inline => Some(Action::Up),
            (KeyCode::Right, _) | (_, Some(K('l'))) if inline => Some(Action::Down),
            (_, Some(K(c @ '1'..='9'))) => Some(Action::Jump(c as u8 - b'1')),
            (_, Some(k)) if binding(Section::is_global, k) == Some(opener) => Some(Action::Keep),
            _ => None,
        },
    }
}

impl Section {
    /// Its keys work with nothing open (all but the player keys).
    fn is_global(self) -> bool {
        self != Music
    }
}

/// What `key` does in the rows of the sections `live` accepts.
fn binding(live: impl Fn(Section) -> bool, key: Key) -> Option<Action> {
    KEYMAP
        .iter()
        .filter(|r| live(r.section))
        .flat_map(|r| r.binds)
        .find(|(k, _)| *k == key)
        .map(|&(_, a)| a)
}

fn mouse_action(mouse: &MouseEvent, mode: InputMode) -> Option<Action> {
    let (col, row) = (mouse.column, mouse.row);
    let list = matches!(
        mode,
        InputMode::Help | InputMode::Picker { .. } | InputMode::Library { .. }
    );
    match (mouse.kind, mode) {
        (MouseEventKind::ScrollUp, _) if list => Some(Action::Up),
        (MouseEventKind::ScrollDown, _) if list => Some(Action::Down),
        (MouseEventKind::Down(MouseButton::Left), InputMode::Normal | InputMode::Player) => {
            Some(Action::Press { col, row })
        }
        (MouseEventKind::Drag(MouseButton::Left), InputMode::Normal) => {
            Some(Action::Poke { col, row })
        }
        (
            MouseEventKind::Down(MouseButton::Left),
            InputMode::Picker { .. } | InputMode::Library { .. },
        ) => Some(Action::Click { col, row }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn ch(c: char) -> Event {
        press(KeyCode::Char(c), KeyModifiers::NONE)
    }

    const PICKER: InputMode = InputMode::Picker {
        opener: Action::StylePicker,
        inline: false,
    };

    #[test]
    fn a_reported_repeat_of_r_never_resets() {
        let kind = |kind| {
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('r'),
                KeyModifiers::NONE,
                kind,
            ))
        };
        let normal = InputMode::Normal;
        assert_eq!(
            action_for(&kind(KeyEventKind::Press), normal),
            Some(Action::PomodoroReset)
        );
        assert_eq!(action_for(&kind(KeyEventKind::Repeat), normal), None);
        assert_eq!(action_for(&kind(KeyEventKind::Release), normal), None);
        // Other held keys still repeat.
        let held = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char(']'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        assert_eq!(action_for(&held, normal), Some(Action::HeatUp));
    }

    #[test]
    fn quit_keys() {
        assert_eq!(action_for(&ch('q'), InputMode::Normal), Some(Action::Quit));
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for mode in [InputMode::Normal, InputMode::Help, PICKER] {
            assert_eq!(action_for(&ctrl_c, mode), Some(Action::Quit));
        }
    }

    #[test]
    fn esc_never_quits() {
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action_for(&esc, InputMode::Normal), None);
        assert_eq!(action_for(&esc, InputMode::Help), Some(Action::Close));
        assert_eq!(action_for(&esc, PICKER), Some(Action::Close));
    }

    #[test]
    fn q_closes_overlays_instead_of_quitting() {
        assert_eq!(action_for(&ch('q'), InputMode::Help), Some(Action::Close));
        assert_eq!(action_for(&ch('q'), PICKER), Some(Action::Close));
    }

    fn event_of(key: Key) -> Event {
        match key {
            Key::Char(c) => ch(c),
            Key::Ctrl(c) => press(KeyCode::Char(c), KeyModifiers::CONTROL),
            Key::Space => ch(' '),
            Key::Left => press(KeyCode::Left, KeyModifiers::NONE),
            Key::Right => press(KeyCode::Right, KeyModifiers::NONE),
            Key::Up => press(KeyCode::Up, KeyModifiers::NONE),
            Key::Down => press(KeyCode::Down, KeyModifiers::NONE),
        }
    }

    #[test]
    fn every_bound_key_dispatches_to_its_row() {
        for row in KEYMAP {
            let mode = if row.section == Section::Music {
                InputMode::Player
            } else {
                InputMode::Normal
            };
            for &(key, action) in row.binds {
                assert_eq!(action_for(&event_of(key), mode), Some(action), "{key:?}");
            }
        }
    }

    #[test]
    fn player_keys_take_the_keyboard_until_esc() {
        let player = InputMode::Player;
        let normal = InputMode::Normal;
        assert_eq!(action_for(&ch('A'), normal), Some(Action::PlayerKeys));
        // Shared letters mean the player's thing in the player keys...
        assert_eq!(
            action_for(&ch(' '), player),
            Some(Action::Player(PlayerKey::PlayPause))
        );
        assert_eq!(
            action_for(&ch('n'), player),
            Some(Action::Player(PlayerKey::Next))
        );
        // ...and the lamp's / pomodoro's outside them.
        assert_eq!(action_for(&ch(' '), normal), Some(Action::PomodoroToggle));
        assert_eq!(action_for(&ch('n'), normal), Some(Action::PomodoroSkip));
        let left = press(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(action_for(&left, normal), None);
        // Leaving: esc, q, A; help still opens; other globals are off.
        for leave in [press(KeyCode::Esc, KeyModifiers::NONE), ch('q'), ch('A')] {
            assert_eq!(action_for(&leave, player), Some(Action::Close));
        }
        assert_eq!(action_for(&ch('?'), player), Some(Action::Help));
        assert_eq!(action_for(&ch('c'), player), None);
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action_for(&ctrl_c, player), Some(Action::Quit));
    }

    #[test]
    fn every_dock_widget_has_a_place_key() {
        let placed: Vec<&str> = KEYMAP
            .iter()
            .flat_map(|r| r.binds)
            .filter_map(|b| match b.1 {
                Action::Place(name) => Some(name),
                _ => None,
            })
            .collect();
        for w in crate::dock::WIDGETS {
            assert!(placed.contains(&w.name()), "no key places {}", w.name());
        }
        for name in placed {
            assert!(
                crate::dock::by_name(name).is_some(),
                "{name} isn't a widget"
            );
        }
    }

    #[test]
    fn keys_are_unique() {
        // Within each mode: the player keys may reuse global ones.
        for player in [false, true] {
            let keys: Vec<Key> = KEYMAP
                .iter()
                .filter(|r| (r.section == Section::Music) == player)
                .flat_map(|r| r.binds)
                .map(|b| b.0)
                .collect();
            for (i, k) in keys.iter().enumerate() {
                assert!(!keys[i + 1..].contains(k), "{k:?} bound twice");
            }
        }
    }

    #[test]
    fn shifted_letters_open_pickers() {
        let shift = |c| press(KeyCode::Char(c), KeyModifiers::SHIFT);
        assert_eq!(
            action_for(&shift('S'), InputMode::Normal),
            Some(Action::StylePicker)
        );
        assert_eq!(
            action_for(&shift('C'), InputMode::Normal),
            Some(Action::FacePicker)
        );
        assert_eq!(
            action_for(&shift('P'), InputMode::Normal),
            Some(Action::PalettePicker)
        );
        assert_eq!(
            action_for(&shift('+'), InputMode::Normal),
            Some(Action::Faster)
        );
    }

    #[test]
    fn picker_keys() {
        assert_eq!(action_for(&ch('j'), PICKER), Some(Action::Down));
        assert_eq!(action_for(&ch('3'), PICKER), Some(Action::Jump(2)));
        assert_eq!(action_for(&ch(' '), PICKER), Some(Action::Keep));
        assert_eq!(action_for(&ch('S'), PICKER), Some(Action::Keep));
        assert_eq!(
            action_for(&ch('s'), PICKER),
            None,
            "globals are off in overlays"
        );
        assert_eq!(action_for(&ch('l'), PICKER), None);
        let inline = InputMode::Picker {
            opener: Action::StylePicker,
            inline: true,
        };
        assert_eq!(action_for(&ch('l'), inline), Some(Action::Down));
        assert_eq!(
            action_for(&press(KeyCode::Left, KeyModifiers::NONE), inline),
            Some(Action::Up)
        );
    }

    #[test]
    fn library_keys() {
        let lib = InputMode::Library { inline: false };
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(action_for(&esc, lib), Some(Action::Back));
        assert_eq!(action_for(&ch('q'), lib), Some(Action::Close));
        assert_eq!(
            action_for(&press(KeyCode::Enter, KeyModifiers::NONE), lib),
            Some(Action::Keep)
        );
        assert_eq!(action_for(&ch('l'), lib), Some(Action::Keep));
        assert_eq!(action_for(&ch('h'), lib), Some(Action::Back));
        assert_eq!(action_for(&ch('p'), lib), Some(Action::PlayAll));
        assert_eq!(action_for(&ch('G'), lib), Some(Action::Edge(true)));
        assert_eq!(
            action_for(&press(KeyCode::PageDown, KeyModifiers::NONE), lib),
            Some(Action::Page(true))
        );
        assert_eq!(action_for(&ch('s'), lib), None, "globals are off");
        let inline = InputMode::Library { inline: true };
        assert_eq!(action_for(&ch('l'), inline), Some(Action::Down));
    }

    #[test]
    fn the_mouse_presses_buttons_and_scrolls_lists() {
        let at = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: 3,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        };
        let down = at(MouseEventKind::Down(MouseButton::Left));
        let press = Some(Action::Press { col: 3, row: 4 });
        assert_eq!(action_for(&down, InputMode::Normal), press);
        assert_eq!(action_for(&down, InputMode::Player), press);
        let lib = InputMode::Library { inline: false };
        assert_eq!(
            action_for(&down, lib),
            Some(Action::Click { col: 3, row: 4 })
        );
        assert_eq!(
            action_for(&at(MouseEventKind::ScrollDown), lib),
            Some(Action::Down)
        );
        assert_eq!(
            action_for(
                &at(MouseEventKind::Drag(MouseButton::Left)),
                InputMode::Normal
            ),
            Some(Action::Poke { col: 3, row: 4 })
        );
    }

    #[test]
    fn events() {
        assert_eq!(
            action_for(&Event::Resize(80, 24), InputMode::Normal),
            Some(Action::Resize)
        );
        assert_eq!(
            action_for(&Event::FocusLost, InputMode::Normal),
            Some(Action::Focus(false))
        );
    }
}
