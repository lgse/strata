// SPDX-License-Identifier: MIT

//! The open columns as the Columns component reads them: the shared listings
//! ([`crate::listing`]), root first, behind strata-components' `ColumnSources`.

use std::borrow::Cow;

use strata_components::{
    columns::ColumnSources,
    contract::{EntryId, EntrySource},
};

use crate::listing::{Column, Words};

/// The open columns, root first, and the words for what a row is.
pub(super) struct Listings {
    pub(super) columns: Vec<Column>,
    described: Words,
}

impl Listings {
    pub(super) fn translated() -> Self {
        Self {
            columns: Vec::new(),
            described: Words::translated(),
        }
    }
}

impl ColumnSources for Listings {
    fn len(&self) -> usize {
        self.columns.len()
    }

    fn source(&self, depth: usize) -> &dyn EntrySource {
        &self.columns[depth]
    }

    fn version(&self, depth: usize) -> u64 {
        self.columns[depth].version()
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

    fn open_child(&self, depth: usize) -> Option<EntryId> {
        self.columns.get(depth + 1)?.from()
    }

    fn description(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let description = self
            .columns
            .get(depth)?
            .description(index, &self.described)?;
        Some(Cow::Borrowed(description))
    }
}
