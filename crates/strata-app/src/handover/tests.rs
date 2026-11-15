// SPDX-License-Identifier: MIT

use std::{path::Path, rc::Rc};

use strata_components::collection::{element::CollectionState, selection::Position};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::{adapters::LocalFileSource, app::Browser, app::BrowserEvent};

use super::Moving;
use crate::{
    events::{self, Change},
    listing::{Column, Edit, Row},
    slots::Ids,
};

fn file_entry(path: &Path) -> FileEntry {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_owned();
    FileEntry {
        location: Location::local(path),
        thumbnail_path: None,
        native_name: name.clone().into(),
        display_name: name,
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Known(0o644),
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn moving_consumes_arrivals_once_and_skips_already_selected_targets() {
    let ids = Ids::default();
    let root = Location::local(Path::new("/tmp/root"));
    let old_entry = file_entry(Path::new("/tmp/root/old.txt"));
    let new_entry = file_entry(Path::new("/tmp/root/new.txt"));
    let from = ids.of(&old_entry.location);
    let to = ids.of(&new_entry.location);

    let mut column = Column::new(root, false, None);
    column.apply(Edit::Replace(vec![Row::of(&old_entry, &ids)]));
    let mut collection = CollectionState::new();
    let _ = collection.selection.restore(&Position {
        cursor: Some(from),
        cursor_index: Some(0),
        anchor: Some(from),
        selected: vec![from],
        provisional: false,
        scroll: None,
    });

    let mut moving = Moving::default();
    moving.moved(from, to, |id| column.position_of(id).is_some());

    column.apply(Edit::Replace(vec![Row::of(&new_entry, &ids)]));
    assert_eq!(moving.arrived(&column, &mut collection), vec![(from, to)]);
    assert!(collection.selection.contains(&column, to));

    // A second edit on the same column must not re-apply the consumed relocation onto `to`.
    assert!(moving.arrived(&column, &mut collection).is_empty());

    // Even if a fresh pair targets an id the selection already holds, `arrived` does not panic.
    let other_from = ids.of(&Location::local(Path::new("/tmp/other/old.txt")));
    moving.moved(other_from, to, |id| id == other_from);
    assert_eq!(
        moving.arrived(&column, &mut collection),
        vec![(other_from, to)]
    );
    assert!(collection.selection.contains(&column, to));
}

#[test]
fn item_relocated_only_emits_change_moved_within_same_parent_folder() {
    let browser = Browser::new(Rc::new(LocalFileSource));
    let ids = Ids::default();

    let same_folder = BrowserEvent::ItemRelocated {
        from: Location::local(Path::new("/tmp/root/a.txt")),
        to: Location::local(Path::new("/tmp/root/b.txt")),
        merged: false,
    };
    let same_read = events::read(&same_folder, &browser, &ids);
    assert!(
        same_read
            .changes
            .iter()
            .any(|change| matches!(change, Change::Moved { .. }))
    );

    let cross_folder = BrowserEvent::ItemRelocated {
        from: Location::local(Path::new("/tmp/root/a.txt")),
        to: Location::local(Path::new("/tmp/root/dest/a.txt")),
        merged: false,
    };
    let cross_read = events::read(&cross_folder, &browser, &ids);
    assert!(cross_read.changes.is_empty());
}
