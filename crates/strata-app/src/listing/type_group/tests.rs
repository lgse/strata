// SPDX-License-Identifier: MIT

//! A row's type, read from its kind and name, is strata-fs's `entry_type` of the whole entry.

use std::path::Path;

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::services::entry_type;

use super::TypeGroup;

const FOLDER: &str = "/folder";

fn whole(kind: EntryKind, name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(Path::new(FOLDER).join(name)),
        thumbnail_path: Some(Path::new("/thumbnails/one.png").into()),
        native_name: name.into(),
        display_name: name.to_owned(),
        kind,
        size: MetadataValue::Known(1024),
        modified_unix_seconds: MetadataValue::Known(1),
        created_unix_seconds: MetadataValue::Known(1),
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Known(0o644),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Known(true),
        is_hidden: false,
    }
}

#[test]
fn a_rows_type_is_entry_types_for_every_kind() {
    let cases = [
        (EntryKind::Directory, "docs"),
        (EntryKind::File, "photo.png"),
        (EntryKind::File, "notes.txt"),
        (EntryKind::File, "unknown.zzzqx"),
        (EntryKind::SymbolicLink, "stale"),
        (EntryKind::Other, "socket"),
        (EntryKind::DirectorySymbolicLink, "shortcut"),
        (EntryKind::FileSymbolicLink, "link.png"),
    ];
    let folder = Location::local(FOLDER);
    for (kind, name) in cases {
        let row = TypeGroup::of_row(&folder, kind, name).description();
        let entry = entry_type(&whole(kind, name));
        assert_eq!(&*row, entry.description(), "{kind:?} {name}");
    }
}
