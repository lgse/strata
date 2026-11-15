// SPDX-License-Identifier: MIT

//! A search result as the window shows one: GTK's `search_result_entry`
//! (`src/ui/browser/collection.rs`), the entry the browser's checks and a row are made from.

use std::collections::HashMap;

use strata_components::contract::{EntryId, EntrySource};
use strata_core::model::{FileEntry, Location, MetadataValue};
use strata_fs::services::SearchItem;

use crate::listing::Column;

/// One delivery of search results to show in a column or pane.
pub(crate) struct HitsUpdate {
    pub(crate) hits: Column,
    /// Clear the results' selection (a changed query over results that replace the listing).
    pub(crate) clear: bool,
    /// A renamed result's `(old_id, new_id)` to relocate before syncing, when `!clear`.
    pub(crate) relocated: Option<(EntryId, EntryId)>,
}

impl HitsUpdate {
    pub(crate) fn plain(hits: Column, clear: bool) -> Self {
        Self {
            hits,
            clear,
            relocated: None,
        }
    }

    /// The `(from, to)` pair to relocate before `self.hits` replaces `old`, when `!self.clear`,
    /// `old` does not already list `to`, and `self.hits` replaced `from` with `to`.
    pub(crate) fn relocation(&self, old: &dyn EntrySource) -> Option<(EntryId, EntryId)> {
        let (from, to) = self.relocated.filter(|_| !self.clear)?;
        let replaced = old.index_of(to).is_none()
            && self.hits.index_of(to).is_some()
            && self.hits.index_of(from).is_none();
        replaced.then_some((from, to))
    }
}

/// The entry result `item` is, with nothing known of it beyond its name, kind and mode.
pub(super) fn entry_of(item: &SearchItem) -> FileEntry {
    FileEntry {
        location: Location::local(item.path.clone()),
        native_name: item.path.file_name().unwrap_or_default().to_os_string(),
        thumbnail_path: None,
        display_name: item.name.clone(),
        kind: item.kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: item.mode.clone(),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

/// Each result's path as the window shows a folder's (`display_path`), by its id: what a
/// Columns result shows under its name (`show_origin`).
pub(super) fn paths(found: &HashMap<EntryId, FileEntry>) -> HashMap<EntryId, String> {
    found
        .iter()
        .map(|(id, entry)| (*id, entry.location.display_path()))
        .collect()
}
