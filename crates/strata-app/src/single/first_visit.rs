// SPDX-License-Identifier: MIT

//! A first visit to a folder, until the user's own input reaches its pane: GTK's
//! `src/ui/browser_modes/first_visit.rs` and its `on_load_finished` in
//! `src/ui/browser_modes/events.rs`. GTK's list view, taking the keyboard with no cursor,
//! focuses its first row; here the first row shown takes the cursor, with nothing selected.
//!
//! That cursor, and the one [`keep_on_first_row`] moves, reaches the browser through
//! `handover::hand_over`, which commits it, where GTK's browser holds no cursor at all. No
//! replay shows a difference: with nothing selected there is nothing for a command or
//! `follow_selection_on_return` to take (finding lgse/strata#2427).

use strata_components::collection::selection::AnchorRule;

use crate::{
    listing::Load,
    pane::{Applied, Memory, Pane},
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
    /// Follows what a change did to `pane`: another folder showing while it is still read
    /// arms the visit. One shown already listed does not, as GTK's `prepare` returns at once
    /// for a column that is not loading: a closed column that shows its parent again. A
    /// relocation of the folder shown (`Change::Relocated`) applies `Within`, so it does not,
    /// as GTK's `arriving` leaves relocations out. A folder back at the position remembered
    /// for it disarms one a step of the same navigation armed (a reset shows nothing before
    /// the new root opens): GTK starts a first visit only when no saved visit is pending.
    pub(super) fn follow(&mut self, applied: Applied, pane: &Pane) {
        match applied {
            Applied::Entered => self.armed = pane.load() == Some(&Load::Loading),
            Applied::Returned => self.armed = false,
            Applied::Within => {}
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
        // The anchor stays missing: GTK's browser holds none for a first visit, so a Shift+click
        // there selects the clicked entry alone (finding lgse/strata#2551).
        let _ = collection
            .selection
            .move_cursor(source, 0, AnchorRule::Keep);
        collection.note_selection(source);
        true
    }
}

/// Keeps the cursor a first visit placed on the first row `pane` shows, while no input has
/// moved it ([`Memory::visit_cursor`]): GTK's browser holds no cursor then, and each of its
/// views rebuilt shows its own first row, so a regroup or a view switch puts it there (finding
/// lgse/strata#2427). The memory follows it there. Only a change of the first row with the same
/// number of rows is a rebuild: an entry added or gone is none, and GTK's list keeps its focus
/// on the same row then (finding lgse/strata#2642, `icons v3` seed `0x5a`: a file written that
/// lists first). A pane that shows anything else (another folder, another cursor, a selection)
/// forgets it. True when the cursor moved.
pub(super) fn keep_on_first_row(pane: &mut Pane, memory: &Memory) -> bool {
    let Some((depth, id)) = memory.visit_cursor() else {
        return false;
    };
    if pane.load() == Some(&Load::Loading) {
        return false;
    }
    let shown = pane.depth();
    let (source, collection) = pane.parts_mut();
    let selection = &collection.selection;
    let untouched = shown == Some(depth) && selection.cursor() == Some(id) && selection.is_empty();
    if !untouched {
        memory.set_visit_cursor(None);
        return false;
    }
    let rows = (source.len(), (!source.is_empty()).then(|| source.id_at(0)));
    let (count, first) = memory.visit_rows();
    memory.set_visit_rows(rows);
    // Only the same rows in another order are a rebuild.
    if rows.0 != count || rows.1 == first || rows.1.is_none_or(|first| first == id) {
        return false;
    }
    let _ = collection
        .selection
        .move_cursor(source, 0, AnchorRule::Keep);
    collection.note_selection(source);
    memory.set_visit_cursor(Some((depth, source.id_at(0))));
    true
}

#[cfg(test)]
mod tests;
