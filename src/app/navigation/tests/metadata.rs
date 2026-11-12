// SPDX-License-Identifier: MIT

use super::super::metadata::apply_metadata_update;
use super::*;

/// The details a metadata fill can supply for an entry.
#[derive(Debug, PartialEq)]
struct Details {
    size: MetadataValue<u64>,
    modified: MetadataValue<i64>,
    mode: MetadataValue<u32>,
    image_dimensions: MetadataValue<(u32, u32)>,
    child_count: MetadataValue<u64>,
    duration: MetadataValue<u64>,
}

fn details(entry: &FileEntry) -> Details {
    Details {
        size: entry.size.clone(),
        modified: entry.modified_unix_seconds.clone(),
        mode: entry.mode.clone(),
        image_dimensions: entry.image_dimensions.clone(),
        child_count: entry.child_count.clone(),
        duration: entry.duration_seconds.clone(),
    }
}

fn unknown_update(path: &str) -> MetadataUpdate {
    MetadataUpdate {
        location: location(path),
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
    }
}

fn metadata_update(path: &str, size: u64, modified: i64) -> MetadataUpdate {
    MetadataUpdate {
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Known(modified),
        created_unix_seconds: MetadataValue::Unknown,
        ..unknown_update(path)
    }
}

fn files(names: &[&str]) -> Vec<FileEntry> {
    names
        .iter()
        .map(|name| file_entry(&format!("/fixture/{name}"), name))
        .collect()
}

fn counts(total: usize, files: usize, folders: usize) -> Option<ColumnEntryCounts> {
    Some(ColumnEntryCounts {
        total,
        files,
        folders,
    })
}

#[test]
fn metadata_fill_updates_values_without_reordering() {
    let mut state = loaded("/fixture", files(&["bravo", "alpha"]));
    let order_before = column_names(&state, 0);

    let applied = state.apply_metadata(
        RequestId(1),
        vec![
            metadata_update("/fixture/alpha", 10, 100),
            metadata_update("/fixture/bravo", 20, 200),
        ],
    );

    assert_eq!(applied, Some((0, vec![0, 1])));
    let entries = &state.columns[0].entries;
    assert_eq!(
        (&entries[0].size, &entries[1].modified_unix_seconds),
        (&MetadataValue::Known(10), &MetadataValue::Known(200))
    );
    assert_eq!(column_names(&state, 0), order_before);
}

#[test]
fn entry_resolution_follows_splices_and_never_retargets_a_removed_location() {
    let root = location("/fixture");
    let target = location("/fixture/bravo");
    let mut state = loaded("/fixture", files(&["alpha", "bravo"]));
    let resolved = state.resolve_entry_at(0, 0, &target).expect("bravo");
    assert_eq!((resolved.0, resolved.1.location), (1, target.clone()));
    state.apply_directory_change(
        0,
        &root,
        DirectoryChange::Upsert(file_entry("/fixture/aardvark", "aardvark")),
    );
    let resolved = state
        .resolve_entry_at(0, 1, &target)
        .expect("shifted bravo");
    assert_eq!((resolved.0, resolved.1.location), (2, target.clone()));
    assert_eq!(
        state.apply_metadata(
            RequestId(1),
            vec![metadata_update("/fixture/bravo", 20, 200)]
        ),
        Some((0, vec![2]))
    );
    state.apply_directory_change(
        0,
        &root,
        DirectoryChange::Remove(location("/fixture/alpha")),
    );
    let resolved = state.resolve_entry_at(0, 2, &target).expect("shifted back");
    assert_eq!(
        (resolved.0, resolved.1.location, resolved.1.size),
        (1, target.clone(), MetadataValue::Known(20))
    );
    state.apply_directory_change(0, &root, DirectoryChange::Remove(target.clone()));
    assert!(state.resolve_entry_at(0, 1, &target).is_none());
    assert!(state.resolve_entry_at(1, 0, &target).is_none());
}

#[test]
fn metadata_fill_for_a_superseded_load_is_dropped() {
    let mut state = loaded("/fixture", files(&["alpha"]));

    for (request, path) in [(2, "/fixture/alpha"), (1, "/fixture/ghost")] {
        assert_eq!(
            state.apply_metadata(RequestId(request), vec![metadata_update(path, 10, 100)]),
            None,
            "{request:?} {path}"
        );
    }
    assert_eq!(state.columns[0].entries[0].size, MetadataValue::Unknown);
}

#[test]
fn positioned_fill_applies_in_place_and_flags_moved_rows() {
    let mut state = loaded("/fixture", files(&["alpha", "bravo", "charlie"]));

    let applied = state.apply_positioned_metadata(
        RequestId(1),
        vec![
            (0, metadata_update("/fixture/alpha", 30, 100)),
            (2, metadata_update("/fixture/charlie", 10, 200)),
        ],
    );
    assert!(
        matches!(applied, Some((0, ref positions, ref stale)) if positions == &[0, 2] && stale.is_empty())
    );
    assert_eq!(state.columns[0].entries[0].size, MetadataValue::Known(30));

    state.columns[0].entries.remove(0);
    let applied = state.apply_positioned_metadata(
        RequestId(1),
        vec![
            (1, metadata_update("/fixture/bravo", 20, 150)),
            (2, metadata_update("/fixture/charlie", 10, 200)),
        ],
    );
    assert!(
        matches!(applied, Some((0, ref positions, ref stale)) if positions.is_empty() && stale.len() == 2)
    );
    assert_eq!(state.columns[0].entries[0].size, MetadataValue::Unknown);
}

#[test]
fn fill_updates_never_clobber_known_fields() {
    let half_known = FileEntry {
        size: MetadataValue::Known(50),
        ..file_entry("/fixture/half", "half")
    };
    let mut state = loaded("/fixture", vec![half_known]);
    let filled = Details {
        size: MetadataValue::Known(50),
        modified: MetadataValue::Known(300),
        mode: MetadataValue::Known(0o100640),
        image_dimensions: MetadataValue::Known((1920, 1080)),
        child_count: MetadataValue::Known(4),
        duration: MetadataValue::Known(83),
    };

    let applied = state.apply_metadata(
        RequestId(1),
        vec![MetadataUpdate {
            modified_unix_seconds: MetadataValue::Known(300),
            created_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Known(0o100640),
            image_dimensions: MetadataValue::Known((1920, 1080)),
            child_count: MetadataValue::Known(4),
            duration_seconds: MetadataValue::Known(83),
            ..unknown_update("/fixture/half")
        }],
    );
    assert!(matches!(applied, Some((0, ref positions)) if positions == &[0]));
    assert_eq!(details(&state.columns[0].entries[0]), filled);

    let applied = state.apply_metadata(RequestId(1), vec![unknown_update("/fixture/half")]);
    assert_eq!(applied, None);
    assert_eq!(details(&state.columns[0].entries[0]), filled);
}

#[test]
fn gap_targets_cover_directory_mtimes_but_never_directory_sizes() {
    let complete_file = FileEntry {
        size: MetadataValue::Known(5),
        modified_unix_seconds: MetadataValue::Known(60),
        created_unix_seconds: MetadataValue::Unknown,
        ..file_entry("/fixture/done", "done")
    };
    let half_file = FileEntry {
        size: MetadataValue::Known(50),
        ..file_entry("/fixture/half", "half")
    };
    let state = loaded(
        "/fixture",
        vec![named_entry("/fixture/sub", "sub"), complete_file, half_file],
    );

    assert_eq!(
        state
            .column_unknown_sort_metadata(0, crate::model::SortKey::Modified)
            .map(|targets| {
                targets
                    .into_iter()
                    .map(|(position, location)| (position, location.display_path()))
                    .collect::<Vec<_>>()
            }),
        Some(vec![
            (0, "/fixture/sub".to_owned()),
            (2, "/fixture/half".to_owned()),
        ])
    );
}

#[test]
fn column_entry_counts_breakdown_and_hidden() {
    let mut state = NavigationState::default();
    assert_eq!(state.column_entry_counts(0), None);

    state.navigate(location("/fixture"), RequestId(1));
    assert_eq!(state.column_entry_counts(0), counts(0, 0, 0));

    let hidden_file = FileEntry {
        is_hidden: true,
        ..file_entry("/fixture/.hidden", ".hidden")
    };
    let mut entries = vec![named_entry("/fixture/folder1", "folder1")];
    entries.extend(files(&["file1.txt", "file2.txt"]));
    entries.push(hidden_file);
    state.apply_batch(RequestId(1), entries);

    assert_eq!(state.column_entry_counts(0), counts(3, 2, 1));

    state.set_show_hidden(true);
    assert_eq!(state.column_entry_counts(0), counts(4, 3, 1));
}

#[test]
fn creation_sort_requests_birth_times_without_refilling_complete_size_metadata() {
    let entries = vec![
        FileEntry {
            size: MetadataValue::Known(5),
            modified_unix_seconds: MetadataValue::Known(60),
            created_unix_seconds: MetadataValue::Unknown,
            ..file_entry("/fixture/missing-birth", "missing-birth")
        },
        FileEntry {
            created_unix_seconds: MetadataValue::Unavailable,
            ..file_entry("/fixture/no-birth", "no-birth")
        },
    ];
    let state = loaded("/fixture", entries);
    let created = state
        .column_unknown_sort_metadata(0, crate::model::SortKey::Created)
        .expect("missing creation time");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].1, location("/fixture/missing-birth"));
    let modified = state
        .column_unknown_sort_metadata(0, crate::model::SortKey::Modified)
        .expect("missing modification time");
    assert_eq!(modified.len(), 1);
    assert_eq!(modified[0].1, location("/fixture/no-birth"));
}

#[test]
fn computer_item_capacity_is_preserved_across_metadata_updates() {
    let mut computer_entry = file_entry("/fixture/drive", "My Drive");
    computer_entry.recent_uri = Some("computer:///drive.link".to_string());
    computer_entry.size = MetadataValue::Known(1_000_000_000);
    computer_entry.child_count = MetadataValue::Known(400_000_000);

    let update = MetadataUpdate {
        size: MetadataValue::Known(50),
        child_count: MetadataValue::Known(12),
        modified_unix_seconds: MetadataValue::Known(12345),
        ..unknown_update("/fixture/drive")
    };

    assert!(apply_metadata_update(&mut computer_entry, &update));
    assert_eq!(computer_entry.size, MetadataValue::Known(1_000_000_000));
    assert_eq!(
        computer_entry.child_count,
        MetadataValue::Known(400_000_000)
    );
    assert_eq!(
        computer_entry.modified_unix_seconds,
        MetadataValue::Known(12345)
    );
}
