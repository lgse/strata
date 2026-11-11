//! [`ColumnSources`]: the host's listings, as data.

use std::borrow::Cow;

use crate::contract::{EntryId, EntrySource};

/// The folders the strip shows, root first: one listing per column, kept by the host, which
/// lists folders, follows changes and decides what opens (Strata's policy). The view reads it
/// by accessor, as rune-kit's Table reads its rows.
///
/// Its length must match [`ColumnsState::len`](super::ColumnsState::len): the host opens a
/// listing and pushes its column in the same handler, and drops them together.
///
/// Used by: hosts (Strata's browser and chooser, the gallery).
pub trait ColumnSources {
    /// How many columns there are.
    fn len(&self) -> usize;

    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Column `depth`'s listing.
    fn source(&self, depth: usize) -> &dyn EntrySource;

    /// Column `depth`'s listing version, as the collection element wants it (C1e's module
    /// docs): changed whenever its order or entries change at the same length.
    fn version(&self, depth: usize) -> u64;

    /// What the column's header shows and assistive technology names its list by: the folder's
    /// name.
    fn title(&self, depth: usize) -> &str;

    /// Whether the column's folder is still loading: its header shows a spinner.
    fn loading(&self, _depth: usize) -> bool {
        false
    }

    /// The entry in column `depth` whose folder column `depth + 1` shows: the open path. `None`
    /// for the last column, or when the next column shows something else (a search).
    fn open_child(&self, depth: usize) -> Option<EntryId>;

    /// What column `depth`'s entry at `index` shows in its size badge, as the host words it:
    /// "No items" or "12 items" for a folder whose count is known, a file's size. `None` (the
    /// default) shows no badge: a folder whose count is unknown, a size not read yet. The
    /// badge shows while the row is hovered, selected or on the open path, and with
    /// [`ColumnsView::show_sizes`](super::ColumnsView::show_sizes) on; assistive technology
    /// hears it as the row's description under the same setting.
    fn badge(&self, _depth: usize, _index: usize) -> Option<Cow<'_, str>> {
        None
    }
}
