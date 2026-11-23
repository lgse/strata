// SPDX-License-Identifier: MIT

use strata_components::{collection::selection::AnchorRule, contract::EntryId};

use crate::{
    listing::Load,
    pane::{Applied, Memory, Pane},
};

#[derive(Debug, Default)]
pub(super) struct FirstVisit {
    armed: bool,
}

impl FirstVisit {
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

    pub(super) fn settle(&mut self, pane: &mut Pane, tx: &mut rune::Tx) -> bool {
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
        if source.is_empty() {
            return false;
        }
        if let Some(cursor) = collection.selection.cursor() {
            if source.id_at(0) == cursor {
                pane.scroll_to_top(tx);
            }
            return false;
        }
        let _ = collection
            .selection
            .move_cursor(source, 0, AnchorRule::Keep);
        collection.note_selection(source);
        pane.scroll_to_top(tx);
        true
    }
}

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
    let before = memory.visit_rows();
    memory.set_visit_rows(rows);
    if !rebuilt(before, rows, id) {
        return false;
    }
    let _ = collection
        .selection
        .move_cursor(source, 0, AnchorRule::Keep);
    collection.note_selection(source);
    memory.set_visit_cursor(Some((depth, source.id_at(0))));
    true
}

fn rebuilt(before: (usize, Option<EntryId>), now: (usize, Option<EntryId>), id: EntryId) -> bool {
    let same_rows = now.0 == before.0;
    let new_first = now.1 != before.1;
    same_rows && new_first && now.1.is_some_and(|first| first != id)
}

#[cfg(test)]
mod tests;
