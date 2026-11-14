// SPDX-License-Identifier: MIT

//! The open columns as the Columns component reads them: the shared listings
//! ([`crate::listing`]), root first, behind strata-components' `ColumnSources`.
//!
//! A filter's results take the place of one column's rows while they show
//! ([`Listings::show_hits`]), as GTK's column swaps its model for the search's hits
//! (`search_recursively` in `src/ui/browser/columns/append/query.rs`): the column's listing
//! stays, and follows the browser underneath.

use std::borrow::Cow;

use strata_components::{
    columns::ColumnSources,
    contract::{EntryId, EntrySource},
};

use crate::{
    details,
    listing::{Column, Words},
};

/// The open columns, root first, and the words for what a row is.
pub(super) struct Listings {
    pub(super) columns: Vec<Column>,
    described: Words,
    /// A filter's results, and the column whose rows they replace.
    hits: Option<(usize, Column)>,
    /// Moves each time results take a column's place or give it back, so the column's
    /// version changes with what it shows.
    swaps: u64,
}

impl Listings {
    pub(super) fn translated() -> Self {
        Self {
            columns: Vec::new(),
            described: Words::translated(),
            hits: None,
            swaps: 0,
        }
    }

    /// What column `depth` shows: a filter's results, or its folder's rows.
    pub(super) fn shown(&self, depth: usize) -> Option<&Column> {
        match &self.hits {
            Some((at, hits)) if *at == depth => Some(hits),
            _ => self.columns.get(depth),
        }
    }

    /// Whether a filter's results show in the place of column `depth`'s rows.
    pub(super) fn hits_at(&self, depth: usize) -> bool {
        self.hits.as_ref().is_some_and(|(at, _)| *at == depth)
    }

    /// Shows `hits` in the place of column `depth`'s rows, or replaces the results shown there.
    pub(super) fn show_hits(&mut self, depth: usize, hits: Column) {
        if !self.hits_at(depth) {
            self.swaps += 1;
        }
        self.hits = Some((depth, hits));
    }

    /// Gives the column the results showed in its rows back. The depth, when there were any.
    pub(super) fn drop_hits(&mut self) -> Option<usize> {
        let (depth, _) = self.hits.take()?;
        self.swaps += 1;
        Some(depth)
    }

    /// Drops the results when the column they showed in goes or shows another folder: column
    /// `from` and every one after it.
    pub(super) fn drop_hits_from(&mut self, from: usize) {
        if self.hits.as_ref().is_some_and(|(at, _)| *at >= from) {
            self.drop_hits();
        }
    }
}

impl ColumnSources for Listings {
    fn len(&self) -> usize {
        self.columns.len()
    }

    fn source(&self, depth: usize) -> &dyn EntrySource {
        match self.shown(depth) {
            Some(column) => column,
            None => &self.columns[depth],
        }
    }

    fn version(&self, depth: usize) -> u64 {
        let version = self.shown(depth).map_or(0, Column::version);
        version.wrapping_add(self.swaps << 40)
    }

    fn title(&self, depth: usize) -> &str {
        self.columns[depth].title()
    }

    /// `column_heading` in `src/ui/browser/columns/append/header.rs`: the folder's path.
    fn title_description(&self, depth: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.columns[depth].location().display_path()))
    }

    fn loading(&self, depth: usize) -> bool {
        self.columns[depth].is_loading()
    }

    /// None while results show in column `depth`: the next column is no folder of theirs.
    fn open_child(&self, depth: usize) -> Option<EntryId> {
        if self.hits_at(depth) {
            return None;
        }
        self.columns.get(depth + 1)?.from()
    }

    /// GTK's size badge (`column_size_text`, `entry_size` in `src/ui/browser_modes`): a
    /// folder's item count, a file's size, or none while the browser does not know it.
    fn badge(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let column = self.columns.get(depth)?;
        let row = column.row_of(column.id_at(index))?;
        let text = details::size_cell(row.is_folder(), row.facts());
        (!text.is_empty()).then_some(Cow::Owned(text))
    }

    fn description(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let description = self.shown(depth)?.description(index, &self.described)?;
        Some(Cow::Borrowed(description))
    }
}
