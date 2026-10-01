//! The keymap (docs/design.md §6): one table, [`KEYMAP`], drives both
//! dispatch and the help overlay, so they can't drift.
//!
//! Global keys only fire when no overlay is open (except `ctrl-c`, which
//! always quits). Overlays have their own small fixed key sets. `esc` never
//! quits: it closes overlays and is a no-op otherwise.

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
    CycleFrame,
    ToggleLighting,
    ToggleClock,
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
    // Not keys.
    /// The terminal was resized: relayout and redraw now.
    Resize,
    Focus(bool),
    /// Mouse click/drag at a screen cell: a heat pulse if it's on the wax.
    Poke {
        col: u16,
        row: u16,
    },
}

/// A key as written in the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Lamp,
    Clock,
    App,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Lamp => "lamp",
            Section::Clock => "clock & pomodoro",
            Section::App => "app",
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
use Section::{App, Clock, Lamp};

pub static KEYMAP: &[Row] = &[
    row(Lamp, "s", "next style", &[(K('s'), A::NextStyle)]),
    row(Lamp, "S", "style picker", &[(K('S'), A::StylePicker)]),
    row(Lamp, "p", "next palette", &[(K('p'), A::NextPalette)]),
    row(Lamp, "P", "palette picker", &[(K('P'), A::PalettePicker)]),
    row(
        Lamp,
        "f",
        "frame: auto/glass/bleed",
        &[(K('f'), A::CycleFrame)],
    ),
    row(Lamp, "l", "lighting", &[(K('l'), A::ToggleLighting)]),
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
    row(Clock, "t", "show/hide clock", &[(K('t'), A::ToggleClock)]),
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
    row(App, "m", "minimal", &[(K('m'), A::ToggleMinimal)]),
    row(App, "b", "status bar", &[(K('b'), A::ToggleStatusBar)]),
    row(App, "d", "debug hud", &[(K('d'), A::DebugHud)]),
    row(App, "ctrl-l", "redraw", &[(Ctrl('l'), A::Redraw)]),
    row(App, "?", "this help", &[(K('?'), A::Help)]),
    row(App, "q", "quit", &[(K('q'), A::Quit), (Ctrl('c'), A::Quit)]),
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
}

pub fn action_for(event: &Event, mode: InputMode) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key_action(key, mode),
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
            binding(key?)
        }
        InputMode::Help => match (code, key) {
            (KeyCode::Esc, _) | (_, Some(K('?') | K('q'))) => Some(Action::Close),
            (KeyCode::Up, _) | (_, Some(K('k'))) => Some(Action::Up),
            (KeyCode::Down, _) | (_, Some(K('j'))) => Some(Action::Down),
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
            (_, Some(k)) if binding(k) == Some(opener) => Some(Action::Keep),
            _ => None,
        },
    }
}

fn binding(key: Key) -> Option<Action> {
    KEYMAP
        .iter()
        .flat_map(|r| r.binds)
        .find(|(k, _)| *k == key)
        .map(|&(_, a)| a)
}

fn mouse_action(mouse: &MouseEvent, mode: InputMode) -> Option<Action> {
    match (mouse.kind, mode) {
        (MouseEventKind::ScrollUp, InputMode::Help | InputMode::Picker { .. }) => Some(Action::Up),
        (MouseEventKind::ScrollDown, InputMode::Help | InputMode::Picker { .. }) => {
            Some(Action::Down)
        }
        (
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left),
            InputMode::Normal,
        ) => Some(Action::Poke {
            col: mouse.column,
            row: mouse.row,
        }),
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

    #[test]
    fn every_bound_key_dispatches_to_its_row() {
        for row in KEYMAP {
            for &(key, action) in row.binds {
                let event = match key {
                    Key::Char(c) => ch(c),
                    Key::Ctrl(c) => press(KeyCode::Char(c), KeyModifiers::CONTROL),
                    Key::Space => ch(' '),
                };
                assert_eq!(
                    action_for(&event, InputMode::Normal),
                    Some(action),
                    "{key:?}"
                );
            }
        }
    }

    #[test]
    fn keys_are_unique() {
        let keys: Vec<Key> = KEYMAP.iter().flat_map(|r| r.binds).map(|b| b.0).collect();
        for (i, k) in keys.iter().enumerate() {
            assert!(!keys[i + 1..].contains(k), "{k:?} bound twice");
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
