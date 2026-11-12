// SPDX-License-Identifier: MIT

//! The one folder a single-pane view shows, for the Icons and List views: the browser's
//! active column, as the GTK views rebuild their pane from `active_depth`
//! (`rebuild_icons` and `rebuild_list` in `src/ui/browser_modes/rebuild.rs`). It knows
//! nothing of a grid or a table: a view keeps its component's own state beside it.
//!
//! - **Events in.** A view's listener queues the changes [`crate::events`] reads, and
//!   [`Pane::apply`] puts the ones for the shown column into its listing and its
//!   [`CollectionState`]. The other columns, which only the Columns view draws, are not kept.
//! - **One cursor and one selection,** kept as the Columns adapter keeps them
//!   (`columns/selection.rs`): the collection interprets keys and the pointer, and the
//!   browser holds the result. [`Pane::publish`] hands it over through
//!   `Browser::set_selection`, as GTK's Icons and List views do for every key, and
//!   [`Pane::apply`] writes to the collection what the browser changed by itself.

use strata_components::{
    collection::{
        element::CollectionState,
        selection::{CursorGone, Listing, Position, SyncRule},
    },
    contract::{Entry, EntryId, EntrySource},
};
use strata_fs::app::Browser;

use crate::{
    events::{Added, Change, Marks, Moved},
    listing::{Column, Edit, Load},
};

/// What a pane shows before the browser has opened a folder: nothing.
struct Nothing;

impl EntrySource for Nothing {
    fn len(&self) -> usize {
        0
    }

    fn id_at(&self, index: usize) -> EntryId {
        unreachable!("an empty listing has no entry {index}")
    }

    fn index_of(&self, _: EntryId) -> Option<usize> {
        None
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        unreachable!("an empty listing has no entry {index}")
    }
}

/// What applying a change did to the folder shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The change was not for the shown folder, or changed only its rows or marks.
    Within,
    /// Another folder shows, or none does.
    Entered,
}

pub(crate) struct Pane {
    /// The collection's key: its rows' keys and its scroll requests start there.
    scope: &'static str,
    /// The browser's column shown, and its listing. None before a folder opens, and after
    /// the browser closed the column.
    shown: Option<(usize, Column)>,
    /// Moves with every folder shown, so two folders of one length never share a version.
    generation: u64,
    pub(crate) collection: CollectionState,
    /// The revision of the collection's selection that the browser holds. Another revision is
    /// a change the view made, which [`Pane::publish`] hands over.
    held: u64,
    /// Entries a rename gave another id, each as its old id and its new one, until the
    /// listing shows the change.
    moving: Vec<(EntryId, EntryId)>,
}

impl Pane {
    pub(crate) fn new(scope: &'static str, collection: CollectionState) -> Self {
        Self {
            scope,
            shown: None,
            generation: 0,
            collection,
            held: 0,
            moving: Vec::new(),
        }
    }

    /// The browser's column shown.
    pub(crate) fn depth(&self) -> Option<usize> {
        self.shown.as_ref().map(|(depth, _)| *depth)
    }

    pub(crate) fn listing(&self) -> Option<&Column> {
        self.shown.as_ref().map(|(_, column)| column)
    }

    /// The listing as the collection reads it: an empty one while no folder shows.
    pub(crate) fn source(&self) -> &dyn EntrySource {
        match &self.shown {
            Some((_, column)) => column,
            None => &Nothing,
        }
    }

    /// The listing and the collection, as the collection's handlers take them.
    pub(crate) fn parts_mut(&mut self) -> (&dyn EntrySource, &mut CollectionState) {
        let source: &dyn EntrySource = match &self.shown {
            Some((_, column)) => column,
            None => &Nothing,
        };
        (source, &mut self.collection)
    }

    /// The collection's listing version: it moves with every change to what shows.
    pub(crate) fn version(&self) -> u64 {
        let version = self.listing().map_or(0, Column::version);
        self.generation.wrapping_shl(40) ^ version
    }

    /// The listing of column `depth`, when it is the one shown.
    fn column(&self, depth: usize) -> Option<&Column> {
        self.shown
            .as_ref()
            .filter(|(shown, _)| *shown == depth)
            .map(|(_, column)| column)
    }

    fn column_mut(&mut self, depth: usize) -> Option<&mut Column> {
        self.shown
            .as_mut()
            .filter(|(shown, _)| *shown == depth)
            .map(|(_, column)| column)
    }

    /// The browser's position of entry `id` in column `depth`.
    pub(crate) fn position(&self, depth: usize, id: EntryId) -> Option<usize> {
        self.column(depth)?.position_of(id)
    }

    /// The entry `step` entries after `id` in column `depth`, as the view lists them.
    pub(crate) fn neighbour(&self, depth: usize, id: EntryId, step: isize) -> Option<EntryId> {
        let column = self.column(depth)?;
        let index = column.index_of(id)?.checked_add_signed(step)?;
        (index < column.len()).then(|| column.id_at(index))
    }

    /// Applies one change the view's listener queued.
    pub(crate) fn apply(&mut self, change: Change, tx: &mut rune::Tx) -> Applied {
        match change {
            Change::Reset => return self.leave(),
            Change::Truncated { len, .. } if self.depth().is_some_and(|depth| depth >= len) => {
                return self.leave();
            }
            // Only a column the browser makes active is shown: opening a folder in place
            // makes the new root active, and history makes the column it restores active.
            Change::Added(added) if added.active => {
                self.enter(added, tx);
                return Applied::Entered;
            }
            Change::Relocated {
                from_depth,
                columns,
            } => self.relocate(from_depth, columns, tx),
            Change::Moved { from, to } => self.moved(from, to),
            within => self.apply_within(within, tx),
        }
        Applied::Within
    }

    fn apply_within(&mut self, change: Change, tx: &mut rune::Tx) {
        match change {
            Change::Rows { depth, edit, marks } => self.edit(depth, edit, marks, tx),
            Change::Loaded { depth, load, marks } => {
                if let Some(column) = self.column_mut(depth) {
                    column.set_load(load);
                    column.set_busy(false);
                    self.settle(&marks, tx);
                }
            }
            Change::Busy { depth, busy } => {
                if let Some(column) = self.column_mut(depth) {
                    column.set_busy(busy);
                }
            }
            Change::Hidden { show_hidden, marks } => {
                let Some(depth) = self.depth() else {
                    return;
                };
                if let Some(column) = self.column_mut(depth) {
                    column.set_show_hidden(show_hidden);
                }
                if let Some(marks) = marks.get(depth) {
                    self.settle(marks, tx);
                }
            }
            Change::Selection {
                depth,
                marks,
                reveal,
                ..
            } if self.column(depth).is_some() => {
                self.adopt(&marks);
                if reveal {
                    self.reveal_cursor(tx);
                }
            }
            // `apply` took the rest, and another column's are not shown.
            _ => {}
        }
    }

    /// No folder shows: the browser closed the column, or a new root follows.
    fn leave(&mut self) -> Applied {
        self.shown = None;
        self.moving.clear();
        self.generation += 1;
        Applied::Entered
    }

    fn enter(&mut self, added: Added, tx: &mut rune::Tx) {
        let Added {
            depth,
            location,
            show_hidden,
            ..
        } = added;
        self.generation += 1;
        self.moving.clear();
        let column = Column::new(location, show_hidden, None);
        self.collection
            .enter(self.scope, &column, None, Listing::Loading, tx);
        self.held = self.collection.selection.revision();
        self.shown = Some((depth, column));
    }

    /// The folder shown, or one above it, moved: the column shows its new name, with every
    /// row read again, since a row's id comes from its location.
    fn relocate(&mut self, from_depth: usize, moved: Vec<Moved>, tx: &mut rune::Tx) {
        let Some(depth) = self.depth().filter(|depth| *depth >= from_depth) else {
            return;
        };
        let Some(moved) = moved.into_iter().nth(depth - from_depth) else {
            return;
        };
        if let Some(column) = self.column_mut(depth) {
            column.relocate(moved.location, None);
            column.apply(Edit::Replace(moved.rows));
        }
        self.settle(&moved.marks, tx);
    }

    fn edit(&mut self, depth: usize, edit: Edit, marks: Option<Marks>, tx: &mut rune::Tx) {
        let Some(column) = self.column_mut(depth) else {
            return;
        };
        column.apply(edit);
        self.arrived();
        match marks {
            Some(marks) => self.settle(&marks, tx),
            None => self.sync(tx),
        }
    }

    /// Brings the collection in step with its rows after they changed.
    fn sync(&mut self, tx: &mut rune::Tx) {
        let Some((_, column)) = &self.shown else {
            return;
        };
        // Where a cursor goes when its entry leaves is the browser's to say: the view only
        // lets go of it.
        let rule = match column.load() {
            Load::Loading => SyncRule::loading(),
            Load::Listed | Load::Failed(_) => SyncRule::complete(CursorGone::Clear),
        };
        let _ = self.collection.sync(self.scope, column, rule, tx);
    }

    /// After the rows changed: the collection catches up with them, and then the browser's
    /// cursor and selection stand. The one exception is a cursor the collection was waiting
    /// to place until the folder was listed, which is the user's: [`Pane::publish`] hands it
    /// to the browser.
    fn settle(&mut self, marks: &Marks, tx: &mut rune::Tx) {
        let before = self.collection.selection.cursor();
        self.sync(tx);
        let placed = before.is_none() && self.collection.selection.cursor().is_some();
        if !placed {
            self.adopt(marks);
        }
    }

    /// Writes the browser's cursor and selection to the collection, unless it shows them
    /// already: a write ends a Shift range the user is extending.
    fn adopt(&mut self, marks: &Marks) {
        let Some((_, column)) = &self.shown else {
            return;
        };
        // A folder still loading may not list an entry the browser already holds.
        let loading = *column.load() == Load::Loading;
        let listed = |id: &EntryId| loading || column.index_of(*id).is_some();
        let cursor = marks.cursor.filter(listed);
        let selected: Vec<EntryId> = marks.selected.iter().copied().filter(listed).collect();
        let provisional = marks.provisional && cursor.is_some_and(|id| selected == [id]);
        let selection = &mut self.collection.selection;
        let shown = selection.cursor() == cursor
            && selection.is_provisional() == provisional
            && selection.len() == selected.len()
            && selected.iter().all(|id| selection.contains(column, *id));
        if !shown {
            let _ = selection.restore(&Position {
                cursor,
                cursor_index: cursor.and_then(|id| column.index_of(id)),
                anchor: cursor,
                selected,
                provisional,
                scroll: None,
            });
            self.collection.note_selection(column);
        }
        self.held = self.collection.selection.revision();
    }

    fn reveal_cursor(&self, tx: &mut rune::Tx) {
        let Some((_, column)) = &self.shown else {
            return;
        };
        if let Some(index) = self.collection.selection.cursor_index(column) {
            self.collection.reveal(self.scope, column, index, tx);
        }
    }

    /// Hands the browser what the view changed since it last agreed with it. Call it before
    /// GLib dispatches and before any call that makes the browser act on its cursor, so the
    /// browser never works from a selection older than the view's.
    pub(crate) fn publish(&mut self, browser: &Browser) {
        let Some((depth, column)) = &self.shown else {
            return;
        };
        let selection = &self.collection.selection;
        let revision = selection.revision();
        if revision == self.held {
            return;
        }
        self.held = revision;
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
            "the browser takes the pane's selection"
        );
        // The user's own choice, as a key or a click is to the GTK views: it replaces a
        // cursor a load placed.
        browser.commit_selection();
        browser.set_selection(*depth, &selected, cursor);
        if let Some(anchor) = selection.anchor().and_then(position) {
            browser.set_selection_anchor(*depth, anchor);
        }
    }

    /// Selects entry `id` of column `depth` alone, with the cursor on it.
    pub(crate) fn select_only(&mut self, depth: usize, id: EntryId) {
        let Some((_, column)) = self.shown.as_ref().filter(|(shown, _)| *shown == depth) else {
            return;
        };
        let Some(index) = column.index_of(id) else {
            return;
        };
        let _ = self.collection.selection.select_only(column, index);
        self.collection.note_selection(column);
    }

    /// One of the browser's operations gave the entry `from` the id `to`, as a rename does.
    /// The listing shows it in a later event, and the selection may only be told once its
    /// listing has changed, so the pair waits for [`Pane::arrived`].
    fn moved(&mut self, from: EntryId, to: EntryId) {
        let lists = |id: EntryId| {
            self.listing()
                .is_some_and(|column| column.position_of(id).is_some())
        };
        if lists(from) && !lists(to) {
            self.moving.push((from, to));
        }
    }

    /// The rows changed, and the selection has not been brought in step with them yet. What
    /// it holds by the old id of an entry that now shows under its new one it holds by the new
    /// one: a range keeps its anchor and its members through a rename.
    fn arrived(&mut self) {
        let Some(column) = self.listing() else {
            return;
        };
        let here = |(from, to): &(EntryId, EntryId)| {
            column.position_of(*to).is_some() && column.position_of(*from).is_none()
        };
        let arrived: Vec<(EntryId, EntryId)> = self.moving.iter().copied().filter(here).collect();
        let listed: Vec<bool> = self
            .moving
            .iter()
            .map(|(from, _)| column.position_of(*from).is_some())
            .collect();
        for (from, to) in arrived {
            let _ = self.collection.selection.relocate(from, to);
        }
        let mut listed = listed.into_iter();
        self.moving.retain(|_| listed.next().unwrap_or(false));
    }
}

/// What the offscreen host's readers ask beside the tree.
#[cfg(any(test, feature = "qa"))]
impl Pane {
    pub(crate) fn load(&self) -> Option<&Load> {
        self.listing().map(Column::load)
    }
}
