// SPDX-License-Identifier: MIT

//! The rename editor's model: the open edit, how it ends, and what the host hears.

use rune::{Key, KeyEvent, TextEdit};

use super::intent::{By, Kept, Polled, RenameIntent};
use super::stem::{Initial, initial_selection};
use crate::collection::element::CollectionState;
use crate::contract::{EntryId, EntryKind, EntrySource, Point};

/// The open edit.
#[derive(Clone, Debug)]
struct Open {
    /// The key of the collection whose row shows the editor.
    scope: String,
    id: EntryId,
    /// The name the edit started from: an unchanged name renames nothing.
    original: String,
    listed: Option<String>,
}

/// The rename editor's state, held by the host and given to `RenameEditor` by accessor.
#[derive(Debug, Default)]
pub struct RenameState {
    /// The field's model.
    pub edit: TextEdit,
    open: Option<Open>,
    error: Option<String>,
    /// The field had the keyboard since the edit opened: losing it now is a blur.
    seen_focus: bool,
    /// The row showed in the viewport since the edit opened: leaving it now is a scroll out.
    seen_shown: bool,
    /// A field menu the host opened holds the edit: the field's lost keyboard isn't a blur.
    hold: Hold,
}

/// The field menu's hold on the edit ([`RenameState::menu_opened`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Hold {
    #[default]
    None,
    Opened,
    /// The menu has the keyboard.
    Lost,
}

impl RenameState {
    /// No edit open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the editor on entry `id` of the collection keyed `scope`, with `name` in the field.
    pub fn begin(
        &mut self,
        scope: &str,
        id: EntryId,
        name: &str,
        kind: EntryKind,
        initial: Initial,
    ) -> Polled {
        let mut out = self.cancel(By::Host);
        let (start, end) = initial_selection(name, kind, initial);
        self.edit = TextEdit::new(name);
        self.edit.set_selection(start, end);
        self.edit.focused = true;
        self.open = Some(Open {
            scope: scope.to_owned(),
            id,
            original: name.to_owned(),
            listed: None,
        });
        self.error = None;
        self.seen_focus = false;
        self.seen_shown = false;
        self.hold = Hold::None;
        out.changed = true;
        out
    }

    /// The entry being renamed in the collection keyed `scope`: what `Collection::rename` reads.
    pub fn renaming_in(&self, scope: &str) -> Option<EntryId> {
        self.open
            .as_ref()
            .filter(|o| o.scope == scope)
            .map(|o| o.id)
    }

    /// The key of the collection the edit is in: the one whose listing and state `RenameState::tick` takes.
    pub fn scope(&self) -> Option<&str> {
        self.open.as_ref().map(|o| o.scope.as_str())
    }

    /// The entry being renamed, in any collection.
    pub fn renaming(&self) -> Option<EntryId> {
        self.open.as_ref().map(|o| o.id)
    }

    /// Whether an edit is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Whether the field has the keyboard.
    pub fn has_keyboard(&self) -> bool {
        self.open.is_some() && self.edit.focused
    }

    /// The host's validator's message for the name in the field, if it refused it.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Ends the edit as Enter would: renames, moves or keeps, by the name in the field.
    pub fn commit(&mut self, by: By) -> Polled {
        let Some(open) = self.open.as_ref() else {
            return Polled::default();
        };
        let name = self.edit.text.clone();
        let intent = if name.is_empty() {
            keep(open.id, Kept::Empty, by)
        } else if name == open.original {
            keep(open.id, Kept::Unchanged, by)
        } else if self.error.is_some() {
            keep(open.id, Kept::Invalid, by)
        } else if name.contains('/') {
            RenameIntent::Move {
                id: open.id,
                path: name,
                by,
            }
        } else {
            RenameIntent::Rename {
                id: open.id,
                name,
                by,
            }
        };
        self.close();
        Polled {
            changed: true,
            intents: vec![intent],
        }
    }

    /// Ends the edit keeping the old name.
    pub fn cancel(&mut self, by: By) -> Polled {
        let Some(open) = self.open.as_ref() else {
            return Polled::default();
        };
        let intent = keep(open.id, Kept::Cancelled, by);
        self.close();
        Polled {
            changed: true,
            intents: vec![intent],
        }
    }

    /// One poll, with the key, listing and collection of the edit's scope (`RenameState::scope`).
    pub fn tick(
        &mut self,
        scope: &str,
        source: &dyn EntrySource,
        coll: &CollectionState,
    ) -> Polled {
        let Some(open) = self.open.as_mut() else {
            return Polled::default();
        };
        debug_assert_eq!(
            scope, open.scope,
            "RenameState::tick takes the listing and collection of the edit's scope"
        );
        let Some(index) = source.index_of(open.id) else {
            return self.cancel(By::Listing);
        };
        let listed = source.entry(index).name;
        match &open.listed {
            None => open.listed = Some(listed.to_owned()),
            Some(seen) if seen != listed => return self.cancel(By::Listing),
            Some(_) => {}
        }
        let shown = coll.shows_entry(index, source.len()) || coll.visible_range().contains(&index);
        if shown {
            self.seen_shown = true;
        } else if self.seen_shown {
            return self.cancel(By::Scrolled);
        }
        if self.edit.focused {
            self.seen_focus = true;
            self.hold = Hold::None;
        } else if self.seen_focus {
            if self.hold == Hold::None {
                return self.commit(By::Blur);
            }
            self.hold = Hold::Lost;
        }
        Polled::default()
    }

    /// The host opened a field menu for a `RenameIntent::FieldMenu`, one that takes the keyboard.
    pub fn menu_opened(&mut self) {
        if self.open.is_some() {
            self.hold = Hold::Opened;
        }
    }

    /// The field menu closed: the field asks for the keyboard back if the menu didn't return it.
    pub fn menu_closed(&mut self) -> Polled {
        if std::mem::take(&mut self.hold) == Hold::None || self.open.is_none() {
            return Polled::default();
        }
        if !self.edit.focused {
            self.edit.focused = true;
            return Polled::changed();
        }
        Polled::default()
    }

    pub(super) fn key(&mut self, ev: &KeyEvent, repeat: bool) -> Option<Polled> {
        if self.open.is_none() || self.edit.is_composing() {
            return None;
        }
        let m = ev.mods;
        let chorded = m.ctrl || m.alt || m.cmd || m.logo;
        let by = match ev.key {
            Key::PageUp | Key::PageDown if !chorded => return Some(Polled::default()),
            Key::Enter if !chorded => By::Enter,
            Key::Escape if !chorded => By::Escape,
            Key::Tab if !chorded => {
                if m.shift {
                    By::Previous
                } else {
                    By::Next
                }
            }
            _ => return None,
        };
        if repeat && matches!(by, By::Enter | By::Escape) {
            return Some(Polled::default());
        }
        Some(match by {
            By::Escape => self.cancel(by),
            _ => self.commit(by),
        })
    }

    /// The validator's answer for the text in the field: the editor calls it as the text changes.
    pub fn validated(&mut self, error: Option<String>) -> bool {
        let error = error.filter(|_| !self.edit.text.is_empty());
        if self.error == error {
            return false;
        }
        self.error = error;
        true
    }

    /// The field asked for its menu.
    pub(super) fn field_menu(&self, at: Point) -> Polled {
        let Some(open) = &self.open else {
            return Polled::default();
        };
        Polled {
            changed: false,
            intents: vec![RenameIntent::FieldMenu { id: open.id, at }],
        }
    }

    fn close(&mut self) {
        self.open = None;
        self.error = None;
        self.seen_focus = false;
        self.seen_shown = false;
        self.hold = Hold::None;
        self.edit = TextEdit::new("");
    }
}

fn keep(id: EntryId, why: Kept, by: By) -> RenameIntent {
    RenameIntent::Keep { id, why, by }
}
