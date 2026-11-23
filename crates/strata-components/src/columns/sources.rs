// SPDX-License-Identifier: MIT

//! [`ColumnSources`]: the host's listings, as data.

use std::borrow::Cow;

use crate::contract::{EntryId, EntrySource};

/// The folders the strip shows, root first: one listing per column, kept by the host, which lists folders.
pub trait ColumnSources {
    /// How many columns there are.
    fn len(&self) -> usize;

    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Column `depth`'s listing.
    fn source(&self, depth: usize) -> &dyn EntrySource;

    /// Column `depth`'s listing version, as the collection element wants it.
    fn version(&self, depth: usize) -> u64;

    /// What the column's header shows and assistive technology names its list by: the folder's name.
    fn title(&self, depth: usize) -> &str;

    /// What assistive technology reads after column `depth`'s header title, as the host words it.
    fn title_description(&self, _depth: usize) -> Option<Cow<'_, str>> {
        None
    }

    /// Whether the column's folder is still loading: its header shows a spinner.
    fn loading(&self, _depth: usize) -> bool {
        false
    }

    /// Whether the column's header spinner shows now.
    fn spinner(&self, depth: usize) -> bool {
        self.loading(depth)
    }

    /// The entry in column `depth` whose folder column `depth + 1` shows: the open path.
    fn open_child(&self, depth: usize) -> Option<EntryId>;

    /// What column `depth`'s entry at `index` shows in its size badge, as the host words it.
    fn badge(&self, _depth: usize, _index: usize) -> Option<Cow<'_, str>> {
        None
    }

    /// What assistive technology reads after the name of column `depth`'s entry at `index`, as the host words it.
    fn description(&self, _depth: usize, _index: usize) -> Option<Cow<'_, str>> {
        None
    }

    /// Whether column `depth`'s rows carry a detail line under their names (`Self::detail`).
    fn has_details(&self, _depth: usize) -> bool {
        false
    }

    /// The detail line under the name of column `depth`'s entry at `index`, as the host words it.
    fn detail(&self, _depth: usize, _index: usize) -> Option<Cow<'_, str>> {
        None
    }

    /// Version of what `Self::tooltip` reads for column `depth`'s entry at `index` beyond its entry and row state (for example, its modified timestamp and active date format).
    fn tooltip_version(&self, _depth: usize, _index: usize) -> u64 {
        0
    }

    /// Hover tooltip text for column `depth`'s entry at `index` when shown unconditionally (for example `"{name}\n{date}"`).
    fn tooltip(&self, _depth: usize, _index: usize) -> Option<Cow<'_, str>> {
        None
    }
}
