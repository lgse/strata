// SPDX-License-Identifier: MIT

use strata_components::{
    collection::{
        element::CollectionState,
        selection::{CursorGone, Position, SyncRule},
    },
    contract::{EntryId, EntrySource, ThumbKey},
};
use strata_fs::app::Browser;

use crate::{
    events::Marks,
    listing::{Column, Load, ThumbOrigin},
};

/// The revision of one collection's selection that the browser holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held(u64);

impl Held {
    /// The browser holds what `collection` shows now.
    pub(crate) fn of(collection: &CollectionState) -> Self {
        Self(collection.selection.revision())
    }
}

pub(crate) fn sync_rule(column: &Column) -> SyncRule {
    match column.load() {
        Load::Loading => SyncRule::loading(),
        Load::Listed | Load::Failed(_) => SyncRule::complete(CursorGone::Clear),
    }
}

pub(crate) fn settle_rule(column: &Column) -> SyncRule {
    match column.load() {
        Load::Loading => SyncRule::loading(),
        Load::Listed | Load::Failed(_) => SyncRule::complete(CursorGone::Neighbor),
    }
}

pub(crate) fn kept_neighbour(
    before: Option<EntryId>,
    column: &Column,
    collection: &CollectionState,
    marks: &Marks,
) -> Option<Marks> {
    let left = before.is_some_and(|id| column.index_of(id).is_none());
    let removed = left && *column.load() != Load::Loading;
    if !removed || !unplaced(marks) {
        return None;
    }
    collection.selection.cursor().map(|id| neighbour(id, marks))
}

/// The browser marks no cursor and selects nothing.
fn unplaced(marks: &Marks) -> bool {
    marks.cursor.is_none() && marks.selected.is_empty()
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Kept(Option<EntryId>);

impl Kept {
    /// The neighbour kept, if any.
    pub(crate) fn id(self) -> Option<EntryId> {
        self.0
    }

    /// Starts keeping the cursor of `marks`, [`kept_neighbour`]'s, or nothing.
    pub(crate) fn keep(&mut self, marks: Option<&Marks>) {
        self.0 = marks.and_then(|marks| marks.cursor);
    }

    pub(crate) fn marks(&mut self, collection: &CollectionState, marks: &Marks) -> Option<Marks> {
        let id = self.0?;
        if !unplaced(marks) || collection.selection.cursor() != Some(id) {
            self.0 = None;
            return None;
        }
        Some(neighbour(id, marks))
    }
}

fn neighbour(id: EntryId, marks: &Marks) -> Marks {
    Marks {
        cursor: Some(id),
        anchor: marks.anchor.or(Some(id)),
        ..Marks::default()
    }
}

pub(crate) fn placed(before: Option<EntryId>, collection: &CollectionState) -> bool {
    before.is_none() && collection.selection.cursor().is_some()
}

pub(crate) fn adopt(column: &Column, collection: &mut CollectionState, marks: &Marks) -> Held {
    // A folder still loading may not list an entry the browser already holds.
    let loading = *column.load() == Load::Loading;
    let listed = |id: &EntryId| loading || column.index_of(*id).is_some();
    let cursor = marks.cursor.filter(listed);
    let selected: Vec<EntryId> = marks.selected.iter().copied().filter(listed).collect();
    let provisional = marks.provisional && cursor.is_some_and(|id| selected == [id]);
    let selection = &mut collection.selection;
    let anchor = marks.anchor.filter(listed);
    let shown = selection.cursor() == cursor
        && selection.anchor() == anchor
        && selection.is_provisional() == provisional
        && selection.len() == selected.len()
        && selected.iter().all(|id| selection.contains(column, *id));
    if !shown {
        let _ = selection.restore(&Position {
            cursor,
            cursor_index: cursor.and_then(|id| column.index_of(id)),
            anchor,
            selected,
            provisional,
            scroll: None,
        });
        collection.note_selection(column);
    }
    Held::of(collection)
}

/// One of the browser's columns as a view shows it: the listing, and the collection over it.
pub(crate) struct Shown<'a> {
    pub(crate) depth: usize,
    pub(crate) column: &'a Column,
    pub(crate) collection: &'a CollectionState,
}

#[cfg(feature = "qa")]
#[derive(Default)]
pub(crate) struct Presented<'a> {
    pub(crate) active: Option<usize>,
    pub(crate) strip: Option<usize>,
    /// The listing the view keeps for each of the browser's columns, root first.
    pub(crate) listings: Vec<&'a Column>,
    /// The columns shown with a cursor and a selection.
    pub(crate) shown: Vec<Shown<'a>>,
    pub(crate) pending: Vec<usize>,
    pub(crate) results: Vec<usize>,
    pub(crate) drawn: Vec<Drawn<'a>>,
    pub(crate) kept: Vec<(usize, EntryId)>,
}

/// A filter's results, drawn in the place of column `depth`.
#[cfg(feature = "qa")]
pub(crate) struct Drawn<'a> {
    pub(crate) depth: usize,
    pub(crate) column: &'a Column,
    pub(crate) collection: &'a CollectionState,
}

pub(crate) fn hand_over(shown: &Shown<'_>, browser: &Browser, held: &mut Held) {
    give(shown, browser, held, Handed::Choice);
}

pub(crate) fn hand_back(shown: &Shown<'_>, browser: &Browser, held: &mut Held) {
    give(shown, browser, held, Handed::Restored);
}

/// Whose change the browser is given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handed {
    Choice,
    /// A remembered position the view brought back.
    Restored,
}

fn give(shown: &Shown<'_>, browser: &Browser, held: &mut Held, handed: Handed) {
    let Shown {
        depth,
        column,
        collection,
    } = *shown;
    let selection = &collection.selection;
    let now = Held::of(collection);
    if *held == now {
        return;
    }
    *held = now;
    let position = |id: EntryId| column.position_of(id);
    let selected: Vec<usize> = selection
        .selected_ids(column)
        .into_iter()
        .filter_map(position)
        .collect();
    let cursor = selection.cursor().and_then(position);
    tracing::trace!(
        depth,
        selected = selected.len(),
        ?cursor,
        "the browser takes the view's selection"
    );
    if handed == Handed::Choice {
        browser.commit_selection();
    }
    browser.set_selection(depth, &selected, cursor);
    match selection.anchor().and_then(position) {
        Some(anchor) => browser.set_selection_anchor(depth, anchor),
        None if handed == Handed::Choice => browser.clear_selection_anchor(depth),
        None => {}
    }
}

#[derive(Debug)]
struct PendingMove {
    from: EntryId,
    to: EntryId,
    thumb: Option<ThumbOrigin>,
}

#[derive(Debug, Default)]
pub(crate) struct Moving {
    pending: Vec<PendingMove>,
    thumbnails: Vec<(ThumbKey, ThumbKey)>,
}

impl Moving {
    #[cfg(test)]
    pub(crate) fn moved(&mut self, from: EntryId, to: EntryId, lists: impl Fn(EntryId) -> bool) {
        if lists(from) && !lists(to) {
            self.pending.push(PendingMove {
                from,
                to,
                thumb: None,
            });
        }
    }

    pub(crate) fn moved_in(&mut self, from: EntryId, to: EntryId, columns: &[Column]) {
        if columns
            .iter()
            .any(|column| column.position_of(to).is_some())
        {
            return;
        }
        if let Some(row) = columns.iter().find_map(|column| column.row_of(from)) {
            self.pending.push(PendingMove {
                from,
                to,
                thumb: row.thumb_origin(),
            });
        }
    }

    pub(crate) fn arrived(
        &mut self,
        column: &Column,
        collection: &mut CollectionState,
    ) -> Vec<(EntryId, EntryId)> {
        let arrived = self.arriving(column);
        for (from, to) in &arrived {
            if !collection.selection.holds(*to) {
                let _ = collection.selection.relocate(*from, *to);
            }
        }
        arrived
    }

    pub(crate) fn arriving(&mut self, column: &Column) -> Vec<(EntryId, EntryId)> {
        let mut arrived = Vec::new();
        let thumbnails = &mut self.thumbnails;
        self.pending.retain(|entry| {
            let here =
                column.position_of(entry.to).is_some() && column.position_of(entry.from).is_none();
            if here {
                arrived.push((entry.from, entry.to));
                if let (Some(origin), Some(row)) = (&entry.thumb, column.row_of(entry.to))
                    && let Some(relocated) = row.relocated_thumbnail(origin)
                {
                    thumbnails.push(relocated);
                }
            }
            !here
        });
        arrived
    }

    /// Thumbnail `(from, to)` key pairs produced by arrived renames since the last call.
    pub(crate) fn take_thumbnails(&mut self) -> Vec<(ThumbKey, ThumbKey)> {
        std::mem::take(&mut self.thumbnails)
    }

    /// Drops the pairs whose old id `listed` no longer finds: they have nothing left to move.
    pub(crate) fn forget(&mut self, listed: impl Fn(EntryId) -> bool) {
        self.pending.retain(|entry| listed(entry.from));
    }

    pub(crate) fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        out.extend(self.pending.iter().flat_map(|entry| [entry.from, entry.to]));
    }
}

#[cfg(test)]
mod tests;
