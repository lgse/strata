// SPDX-License-Identifier: MIT

//! What the editor asks of its host.

use crate::contract::{EntryId, Point};

/// What the editor asks of its host.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RenameIntent {
    /// The edit closed on a valid name that differs from the old one and holds no `/`: rename entry `id` to `name`.
    Rename {
        /// The entry.
        id: EntryId,
        /// The new name, as typed (leading and trailing spaces kept).
        name: String,
        /// What closed the edit.
        by: By,
    },
    /// The edit closed on a valid name holding a `/`, which the host's validator accepted.
    Move {
        /// The entry.
        id: EntryId,
        /// The path, as typed.
        path: String,
        /// What closed the edit.
        by: By,
    },
    /// The edit closed and the entry keeps its name.
    Keep {
        /// The entry.
        id: EntryId,
        /// Why nothing is renamed.
        why: Kept,
        /// What closed the edit.
        by: By,
    },
    /// A right-click, the Menu key or Shift+F10 on the field: the host's field menu, at `at` in window points.
    FieldMenu {
        /// The entry.
        id: EntryId,
        /// Where the menu opens, in window points.
        at: Point,
    },
}

impl RenameIntent {
    /// The entry the intent is about.
    pub fn id(&self) -> EntryId {
        match self {
            Self::Rename { id, .. }
            | Self::Move { id, .. }
            | Self::Keep { id, .. }
            | Self::FieldMenu { id, .. } => *id,
        }
    }

    /// What closed the edit, for every intent but [`RenameIntent::FieldMenu`].
    pub fn by(&self) -> Option<By> {
        match self {
            Self::Rename { by, .. } | Self::Move { by, .. } | Self::Keep { by, .. } => Some(*by),
            Self::FieldMenu { .. } => None,
        }
    }
}

/// What closed an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum By {
    /// Enter, or Shift+Enter.
    Enter,
    /// Escape.
    Escape,
    /// The field lost the keyboard: a press elsewhere, another field, autoscroll shield.
    Blur,
    /// Tab: the host moves the editor to the next entry, unless this one is the last.
    Next,
    /// Shift+Tab: the host moves the editor to the previous entry, unless this one is the first.
    Previous,
    /// The host: `RenameState::commit`, `cancel`, or a `begin` while an edit was open.
    Host,
    /// The listing changed under the edit: the entry left it, or its name changed (renamed elsewhere).
    Listing,
    /// The row scrolled out of the viewport.
    Scrolled,
}

impl By {
    /// Whether this ending moves the editor on to a neighbour: `By::Next` (`1`) or `By::Previous` (`-1`).
    pub fn step(self) -> Option<isize> {
        match self {
            Self::Next => Some(1),
            Self::Previous => Some(-1),
            _ => None,
        }
    }
}

/// Why an edit kept the old name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kept {
    /// Escape, the host's cancel, the listing changing, or the row scrolling out.
    Cancelled,
    /// The name is the old one.
    Unchanged,
    /// The field is empty.
    Empty,
    /// The host's validator refused the name.
    Invalid,
}

/// What a call into `RenameState` did: whether the view must be built again, and the intents for the host.
#[derive(Clone, Debug, Default, PartialEq)]
#[must_use]
pub struct Polled {
    /// The view shows something new.
    pub changed: bool,
    /// The intents, in order.
    pub intents: Vec<RenameIntent>,
}

impl Polled {
    pub(super) fn changed() -> Self {
        Self {
            changed: true,
            intents: Vec::new(),
        }
    }

    /// Adds `other`'s change and intents after this one's.
    pub fn merge(&mut self, other: Polled) {
        self.changed |= other.changed;
        self.intents.extend(other.intents);
    }
}
