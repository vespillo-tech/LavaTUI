//! Keymap: raw terminal events → app actions.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Draw a frame now (e.g. the terminal was resized).
    Redraw,
}

pub fn action_for(event: &Event) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => key_action(key),
        // `Terminal::draw` picks up the new size itself; we just redraw now.
        Event::Resize(..) => Some(Action::Redraw),
        _ => None,
    }
}

fn key_action(key: &KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn quit_keys() {
        for event in [
            press(KeyCode::Char('q'), KeyModifiers::NONE),
            press(KeyCode::Esc, KeyModifiers::NONE),
            press(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(action_for(&event), Some(Action::Quit));
        }
    }

    #[test]
    fn plain_c_does_not_quit() {
        assert_eq!(
            action_for(&press(KeyCode::Char('c'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn resize_redraws() {
        assert_eq!(action_for(&Event::Resize(80, 24)), Some(Action::Redraw));
    }
}
