//! The style, clock-face and palette pickers (§4.4): what they list, and
//! the keys and clicks they take while open. The cursor previews live;
//! esc puts back what was there.

use std::time::Duration;

use super::{Model, Overlay};
use crate::clock;
use crate::config::Overridden;
use crate::render::StyleId;
use crate::theme::{Palette, Theme};
use crate::ui::keymap::Action;
use crate::ui::picker::{self, Hit, Placement};

/// Two clicks on the same picker item this close together keep it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Style,
    Face,
    Palette,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            PickerKind::Style => "style",
            PickerKind::Face => "clock",
            PickerKind::Palette => "palette",
        }
    }

    pub fn items(self) -> Vec<&'static str> {
        match self {
            PickerKind::Style => StyleId::all().map(|s| s.style().name()).collect(),
            PickerKind::Face => clock::FACES.iter().map(|f| f.name()).collect(),
            PickerKind::Palette => Palette::all().iter().map(|p| p.name).collect(),
        }
    }

    pub(super) fn opener(self) -> Action {
        match self {
            PickerKind::Style => Action::StylePicker,
            PickerKind::Face => Action::FacePicker,
            PickerKind::Palette => Action::PalettePicker,
        }
    }
}

/// An open picker: the cursor previews live; `original` is what esc restores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    pub cursor: usize,
    pub original: usize,
    /// First item row shown in a sheet; follows the cursor.
    pub top: usize,
}

impl Model {
    /// Returns whether the action was a picker action.
    pub(super) fn picker_action(&mut self, mut picker: Picker, action: Action) -> bool {
        let n = picker.kind.items().len();
        // The opening key again keeps and closes.
        let action = if action == picker.kind.opener() {
            Action::Keep
        } else {
            action
        };
        match action {
            Action::Up => picker.cursor = (picker.cursor + n - 1) % n,
            Action::Down => picker.cursor = (picker.cursor + 1) % n,
            Action::Jump(i) if usize::from(i) < n => picker.cursor = usize::from(i),
            Action::Jump(_) => return true,
            Action::Click { col, row } => {
                let now = self.now;
                match picker::hit(self.layout.area, &self.layout, &picker, col, row) {
                    Some(Hit::Prev) => picker.cursor = (picker.cursor + n - 1) % n,
                    Some(Hit::Next) => picker.cursor = (picker.cursor + 1) % n,
                    Some(Hit::Item(i)) => {
                        let double = self
                            .last_click
                            .is_some_and(|(j, at)| j == i && now - at < DOUBLE_CLICK);
                        if double {
                            self.last_click = None;
                            return self.picker_action(
                                Picker {
                                    cursor: i,
                                    ..picker
                                },
                                Action::Keep,
                            );
                        }
                        self.last_click = Some((i, now));
                        picker.cursor = i;
                    }
                    // Off the picker: swallowed, like any other key.
                    None => return true,
                }
            }
            Action::Keep => {
                self.overlay = Overlay::None;
                self.persist_pick(picker.kind);
                return true;
            }
            Action::Close => {
                self.apply_pick(picker.kind, picker.original);
                self.overlay = Overlay::None;
                return true;
            }
            _ => return false,
        }
        self.apply_pick(picker.kind, picker.cursor);
        self.overlay = Overlay::Picker(self.follow(picker));
        true
    }

    /// Scroll a sheet's list so the cursor stays in view.
    fn follow(&self, mut picker: Picker) -> Picker {
        let area = self.layout.area;
        if let Some(Placement::Sheet { list, .. }) = picker::placement(area, &self.layout, &picker)
        {
            let n = picker.kind.items().len();
            picker.top =
                picker::visible_top(picker.top, picker.cursor, usize::from(list.height), n);
        }
        picker
    }

    pub(super) fn open_picker(&mut self, kind: PickerKind) {
        let i = self.current(kind);
        self.last_click = None;
        self.overlay = Overlay::Picker(self.follow(Picker {
            kind,
            cursor: i,
            original: i,
            top: 0,
        }));
    }

    /// Index of the active item of a picker's kind.
    pub fn current(&self, kind: PickerKind) -> usize {
        match kind {
            PickerKind::Style => self.style.index(),
            PickerKind::Face => clock::FACES
                .iter()
                .position(|f| f.name() == self.face.name())
                .unwrap_or(0),
            PickerKind::Palette => Palette::all()
                .iter()
                .position(|p| p.name == self.theme.palette().name)
                .unwrap_or(0),
        }
    }

    /// Make item `i` live (preview; not yet saved).
    pub(super) fn apply_pick(&mut self, kind: PickerKind, i: usize) {
        match kind {
            PickerKind::Style => {
                if let Some(id) = StyleId::all().nth(i) {
                    self.style = id;
                }
            }
            PickerKind::Face => {
                if let Some(face) = clock::FACES.get(i) {
                    self.face = *face;
                }
            }
            PickerKind::Palette => {
                if let Some(palette) = Palette::all().get(i) {
                    self.theme = Theme::new(palette, self.theme.depth());
                }
            }
        }
    }

    /// Record the live item of `kind` in the settings and schedule a save.
    pub(super) fn persist_pick(&mut self, kind: PickerKind) {
        let now = self.now;
        match kind {
            PickerKind::Style => {
                self.settings.lamp.style = self.style.style().name().into();
                self.overridden.retain(|o| *o != Overridden::Style);
            }
            PickerKind::Face => self.settings.clock.face = self.face.name().into(),
            PickerKind::Palette => {
                self.settings.theme.palette = self.theme.palette().name.into();
                self.overridden.retain(|o| *o != Overridden::Palette);
            }
        }
        self.changed(now);
    }

    pub(super) fn toast_cycle(&mut self, kind: PickerKind) {
        let items = kind.items();
        let i = self.current(kind);
        self.toast(format!("{}  {}/{}", items[i], i + 1, items.len()));
    }
}
