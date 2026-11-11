//! The slow-click rename: armed by a click on the name of the only selected entry, it renames
//! one double-click interval later unless anything happened in between.

use crate::collection::selection::Selection;
use crate::contract::{EntryId, EntrySource};

/// A rename waiting out the double-click interval, as Strata's `schedule_click_rename`
/// (`src/ui/browser/inline_edit.rs`) does: a second press inside it makes a double click
/// instead.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingRename {
    pub(super) id: EntryId,
    /// When it renames, on the host's clock.
    pub(super) due: f64,
    /// The selection's revision once the click was handled: any change after it cancels.
    pub(super) revision: u64,
}

/// What a pending rename does on a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    /// Not yet: keep waiting.
    Wait,
    /// Something happened since it armed: drop it.
    Cancelled,
    /// Rename the entry.
    Rename(EntryId),
}

impl PendingRename {
    /// Whether it renames at `now`. It renames only the same, solely selected entry under the
    /// cursor, as Strata's `click_rename_still_wanted` checks; whether something else is being
    /// edited is the host's to check when it hears the rename.
    pub(super) fn due(&self, now: f64, selection: &Selection, source: &dyn EntrySource) -> Due {
        if selection.revision() != self.revision {
            return Due::Cancelled;
        }
        if now < self.due {
            return Due::Wait;
        }
        let still = selection.len() == 1
            && selection.cursor() == Some(self.id)
            && selection.contains(source, self.id)
            && source.index_of(self.id).is_some();
        if still {
            Due::Rename(self.id)
        } else {
            Due::Cancelled
        }
    }
}
