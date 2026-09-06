// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsString;

use super::*;
use crate::model::{Location, MetadataValue};

fn entry(kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local("/tmp/entry"),
        native_name: OsString::from("entry"),
        thumbnail_path: None,
        display_name: "entry".to_owned(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn every_entry_kind_has_a_spoken_name() {
    let kinds = [
        EntryKind::Directory,
        EntryKind::DirectorySymbolicLink,
        EntryKind::File,
        EntryKind::FileSymbolicLink,
        EntryKind::SymbolicLink,
        EntryKind::Other,
    ];
    for kind in kinds {
        assert!(!entry_kind_name(&entry(kind)).is_empty());
    }
}

#[test]
fn directories_and_files_are_distinguishable() {
    assert_eq!(entry_kind_name(&entry(EntryKind::Directory)), "Folder");
    assert_eq!(entry_kind_name(&entry(EntryKind::File)), "File");
    assert_ne!(
        entry_kind_name(&entry(EntryKind::SymbolicLink)),
        entry_kind_name(&entry(EntryKind::FileSymbolicLink))
    );
}

#[test]
fn each_presentation_has_a_distinct_view_name() {
    let names = [
        view_name(BrowserMode::Columns),
        view_name(BrowserMode::Icons),
        view_name(BrowserMode::List),
    ];
    assert_eq!(names.len(), std::collections::BTreeSet::from(names).len());
}
