// SPDX-License-Identifier: MIT

use super::*;

fn entry(location: Location) -> FileEntry {
    FileEntry {
        location,
        native_name: "fixture".into(),
        thumbnail_path: None,
        display_name: "fixture".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    }
}

#[test]
fn prepared_selection_rejects_changed_targets() {
    let location = Location::local("/fixture/alpha.txt");
    let selection = OpenWithSelection {
        locations: vec![location.clone()],
        files: vec![gio_file_for_location(&location)],
        recommended_apps: vec![],
        other_apps: vec![],
        default: None,
    };
    assert!(selection.entries_match_target(&[entry(location)]));
    assert!(!selection.entries_match_target(&[]));
    assert!(!selection.entries_match_target(&[entry(Location::local("/fixture/beta.txt"))]));
}
