// SPDX-License-Identifier: MIT

use crate::collection::selection::Selection;
use crate::contract::{EntryId, EntrySource};

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
