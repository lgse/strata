// SPDX-License-Identifier: MIT

//! A first visit to a folder, until the user's own input reaches its pane: GTK's
//! `src/ui/browser_modes/first_visit.rs` and its `on_load_finished` in
//! `src/ui/browser_modes/events.rs`. GTK's list view, taking the keyboard with no cursor,
//! focuses its first row; here the first row shown takes the cursor, with nothing selected.

use strata_components::collection::selection::AnchorRule;

use crate::{
    listing::Load,
    pane::{Applied, Pane},
};

/// Armed when another folder shows, until its load ends or the user's input reaches the
/// pane: a press, a wheel or a key (GTK's `is_user_input`), so a user who scrolled or moved
/// keeps the view where they left it.
///
/// It acts only once the listing is no longer loading. The cursor a load places (Return's
/// first entry, Alt+Up's folder the user came from, Back's restored selection) comes with the
/// browser's `Loaded` marks, after the rows: acting on the first rows would beat it, and hand
/// the browser row 0 as the user's own choice at the next publish. By then the file-type
/// groups are final too, so the first row shown is the first of the first group: GTK, which
/// focuses as the first rows come, needs `grouped_focus_drifted` to put that right.
#[derive(Debug, Default)]
pub(super) struct FirstVisit {
    armed: bool,
}

impl FirstVisit {
    /// Follows what a change did to the pane: another folder showing arms the visit. A
    /// relocation of the folder shown (`Change::Relocated`) applies `Within`, so it does not,
    /// as GTK's `arriving` leaves relocations out.
    pub(super) fn follow(&mut self, applied: Applied) {
        if applied == Applied::Entered {
            self.armed = true;
        }
    }

    /// The user's input reached the pane.
    pub(super) fn interrupt(&mut self) {
        self.armed = false;
    }

    /// Ends the visit once the folder shown is listed, or could not be read. A listed folder
    /// with rows and no cursor the browser or the user placed gives its first row shown the
    /// cursor. True when it did. A folder still loading leaves the visit armed, and an empty
    /// one ends it: a file that appears later takes no cursor.
    pub(super) fn settle(&mut self, pane: &mut Pane) -> bool {
        if !self.armed {
            return false;
        }
        match pane.load() {
            Some(Load::Listed) => self.armed = false,
            Some(Load::Failed(_)) => {
                self.armed = false;
                return false;
            }
            Some(Load::Loading) | None => return false,
        }
        let (source, collection) = pane.parts_mut();
        if collection.selection.cursor().is_some() || source.is_empty() {
            return false;
        }
        let _ = collection
            .selection
            .move_cursor(source, 0, AnchorRule::Move);
        collection.note_selection(source);
        true
    }
}

#[cfg(test)]
mod tests;
