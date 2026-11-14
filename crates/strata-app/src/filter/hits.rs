// SPDX-License-Identifier: MIT

//! A search result as the window shows one: GTK's `search_result_entry`
//! (`src/ui/browser/collection.rs`), the entry the browser's checks and a row are made from.

use strata_core::model::{FileEntry, Location, MetadataValue};
use strata_fs::services::SearchItem;

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
