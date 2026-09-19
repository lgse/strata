// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn entry(location: crate::model::Location) -> FileEntry {
    FileEntry {
        location,
        native_name: "file.txt".into(),
        thumbnail_path: None,
        display_name: "file.txt".into(),
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
fn restore_flight_releases_only_when_every_entry_lives_in_trash() {
    let trashed = [
        entry(crate::model::Location::uri("trash:///a.txt")),
        entry(crate::model::Location::uri("trash:///b.txt")),
    ];
    assert_eq!(restore_flight(&trashed), Flight::Release);

    let restored = [entry(crate::model::Location::local("/home/user/a.txt"))];
    assert_eq!(restore_flight(&restored), Flight::Outbound);

    let mixed = [
        entry(crate::model::Location::uri("trash:///a.txt")),
        entry(crate::model::Location::local("/home/user/b.txt")),
    ];
    assert_eq!(restore_flight(&mixed), Flight::Outbound);
}

#[test]
fn release_end_rises_above_the_row_and_fans_out() {
    let row = (300.0, 400.0);
    let single = release_end(row, 0, 1);
    assert_eq!(single.0, row.0);
    assert!(single.1 < row.1);

    let ends: Vec<_> = (0..3).map(|index| release_end(row, index, 3)).collect();
    assert!(ends[0].0 < ends[1].0 && ends[1].0 < ends[2].0);
    assert_eq!(ends[1].0, row.0);
    for end in &ends {
        assert!(end.1 < row.1);
    }

    let near_top = release_end((300.0, 60.0), 0, 1);
    assert!(near_top.1 <= 60.0 - 90.0);
}
