// SPDX-License-Identifier: MIT

//! Where a folder was left, brought back when it shows again: GTK's `PaneNavigation`
//! (`src/ui/browser_modes/navigation.rs`), on C1e's position restore.
//!
//! - **Remembered on leaving** a folder that had finished loading (GTK's `loaded_location`):
//!   its cursor, anchor and selection by entry id, and its scroll offset in the view's layout,
//!   for the 128 folders left most recently (C1e's [`RestoreMemory`], GTK's `HISTORY_LIMIT`).
//! - **Recalled when it shows again loading,** by any navigation, as GTK prepares a restore
//!   whenever a folder in its history starts loading (`prepare` returns at once for a
//!   snapshot not loading): a column already listed that shows again (a deeper one closed)
//!   keeps what the browser holds. C1e brings the selection back once the listing
//!   is complete, and the offset only in the layout it was measured in (GTK: the same mode,
//!   and in Icons the same page width); otherwise it reveals the cursor. Input in the
//!   collection before then cancels it, as GTK's `arm_input_cancel` does.
//! - **An explicit reveal wins** (`reveal_pending`, as GTK's `prepare` cancels for it): the
//!   remembered position stays for a later visit.
//! - **By id.** A view index holds only against the order the listing has now (a grouped
//!   folder shows in its groups' order); C1e's [`Position`] keeps ids, so the restore finds
//!   the same entries whatever order they show in.

use strata_components::collection::{
    element::CollectionState,
    selection::{Position, RestoreMemory},
};
use strata_core::model::Location;

use crate::listing::Column;

#[derive(Default)]
pub(super) struct Restore {
    memory: RestoreMemory<Location>,
    /// The folder being left by the change applying now, and where it was.
    leaving: Option<(Location, Position)>,
    /// The folder shown last came back at a position remembered for it.
    returned: bool,
}

impl Restore {
    /// A change that may show another folder starts: the folder shown now, if it finished
    /// loading, is where `collection` stands.
    pub(super) fn leave(&mut self, shown: Option<&Column>, collection: &CollectionState) {
        self.returned = false;
        self.leaving = shown
            .filter(|column| !column.is_loading())
            .map(|column| (column.location().clone(), collection.position(column)));
    }

    /// Another folder shows, `column` or none: the folder left is remembered, and the
    /// position remembered for `column`, unless the browser reveals an entry there.
    pub(super) fn arrive(&mut self, column: Option<&Column>, reveal: bool) -> Option<Position> {
        if let Some((location, position)) = self.leaving.take() {
            let _ = self.memory.remember(location, position);
        }
        let position = column
            .filter(|column| !reveal && column.is_loading())
            .and_then(|column| self.memory.recall(column.location()))
            .cloned();
        self.returned = position.is_some();
        position
    }

    /// Whether the folder shown last came back at its remembered position.
    pub(super) fn returned(&self) -> bool {
        self.returned
    }
}
