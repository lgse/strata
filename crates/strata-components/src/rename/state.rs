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
    /// The entry's name in the listing when the edit first saw it there: a change means the
    /// entry was renamed elsewhere.
    listed: Option<String>,
}

/// The rename editor's state, held by the host and given to [`RenameEditor`] by accessor.
/// One per window: at most one edit is open.
///
/// The host calls [`RenameState::tick`] on every poll, with the listing and collection of
/// the edit's scope. Each call returns a [`Polled`]: build the view again when it changed,
/// and act on its intents.
///
/// Used by: hosts (C2, C3, C4, Strata).
///
/// [`RenameEditor`]: super::RenameEditor
#[derive(Debug, Default)]
pub struct RenameState {
    /// The field's model. Always present, so a Rune accessor never finds nothing, even for a
    /// field just removed (upstream #180); while no edit is open it is an empty spare. Read it,
    /// but end edits through the methods.
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
    /// The host opened a menu, which hasn't been seen to take the keyboard yet. A menu that
    /// takes it (rune-kit's, C21's) has it by the first `tick` after: it opens and takes it in
    /// the commit of the press or key that asked for it, which runs before the next poll.
    Opened,
    /// The menu has the keyboard.
    Lost,
}

impl RenameState {
    /// No edit open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the editor on entry `id` of the collection keyed `scope`, with `name` in the
    /// field, its stem (or all of it) selected, and the keyboard asked for. An edit already
    /// open is cancelled first ([`By::Host`]).
    ///
    /// The host reveals the row (`CollectionState::reveal`), which also works for a row not
    /// mounted yet.
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

    /// The entry being renamed in the collection keyed `scope`: what C1e's
    /// `Collection::rename` reads.
    pub fn renaming_in(&self, scope: &str) -> Option<EntryId> {
        self.open
            .as_ref()
            .filter(|o| o.scope == scope)
            .map(|o| o.id)
    }

    /// The key of the collection the edit is in: the one whose listing and state
    /// [`RenameState::tick`] takes.
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

    /// Whether the field has the keyboard. While it does, the host leaves alone the keys
    /// the field passes on: its view shortcuts (Ctrl+1–3) and its rename keys (F2, Ctrl+R),
    /// as Strata's do.
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

    /// One poll, with the key, listing and collection of the edit's scope
    /// ([`RenameState::scope`]). It ends an edit whose entry left the listing or was renamed
    /// elsewhere ([`By::Listing`]), whose row scrolled out of the viewport after it had shown
    /// there ([`By::Scrolled`], as Strata's editor ends when its row unmaps), or whose field
    /// lost the keyboard ([`By::Blur`], a commit: Rune has no focus-change hook, upstream
    /// #106).
    ///
    /// Passing another collection's state would judge the row against the wrong viewport: a
    /// debug build panics when `scope` isn't the edit's.
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
        // Rune keeps a focused row mounted out of view, so the field may still have the
        // keyboard: the viewport decides. The check comes before the blur's, as the row also
        // loses the keyboard once Rune unmounts it.
        if coll.visible_range().contains(&index) {
            self.seen_shown = true;
        } else if self.seen_shown {
            return self.cancel(By::Scrolled);
        }
        if self.edit.focused {
            self.seen_focus = true;
            // Either the menu gave the keyboard back (closed, whether or not the host said
            // so), or it never took it (it isn't one that takes it, or the host opened none):
            // either way, nothing holds the edit any more.
            self.hold = Hold::None;
        } else if self.seen_focus {
            if self.hold == Hold::None {
                return self.commit(By::Blur);
            }
            self.hold = Hold::Lost;
        }
        Polled::default()
    }

    /// The host opened a field menu for a [`RenameIntent::FieldMenu`], one that takes the
    /// keyboard (rune-kit's, C21's): until [`RenameState::menu_closed`], the field losing the
    /// keyboard to it ends nothing. Call it from the intent's handler, so the menu opens in the
    /// same commit. The hold also ends on any [`RenameState::tick`] that finds the field with
    /// the keyboard: the menu gave it back, or never took it.
    pub fn menu_opened(&mut self) {
        if self.open.is_some() {
            self.hold = Hold::Opened;
        }
    }

    /// The field menu closed: the field asks for the keyboard back if the menu didn't
    /// return it. A fallback: Rune's modal restore normally gives it back first.
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

    /// A key in the field, before its editor: Enter and Shift+Enter commit, Escape and
    /// Shift+Escape cancel (Shift changes neither, as it changes neither in Strata's entry),
    /// Tab and Shift+Tab commit toward the next or previous entry, and Page Up and Page Down
    /// (with or without Shift) do nothing. `None` passes the key on. Keys an input method is
    /// composing with stay the field's (Rune already gives Enter and Escape to the
    /// composition).
    ///
    /// The page keys stay in the field, as Up, Down, Home and End already do (its editor
    /// takes those): passed on, Rune would page the listing (`runtime/input/keys.rs`,
    /// `page_scroll`), the row would leave the viewport and the edit would end, dropping the
    /// typed name.
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
        // A held Enter or Escape ends one edit; Tab repeats walk the editor along, as
        // Strata's do.
        if repeat && matches!(by, By::Enter | By::Escape) {
            return Some(Polled::default());
        }
        Some(match by {
            By::Escape => self.cancel(by),
            _ => self.commit(by),
        })
    }

    /// The validator's answer for the text in the field: the editor calls it as the text
    /// changes; a host that changes the text itself calls it too. An empty field is never an
    /// error, as Strata's isn't. True when the answer changed.
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
        // A fresh spare: the old text and its history go, and Rune's focus sync for the
        // removed field lands here (#180).
        self.edit = TextEdit::new("");
    }
}

fn keep(id: EntryId, why: Kept, by: By) -> RenameIntent {
    RenameIntent::Keep { id, why, by }
}
