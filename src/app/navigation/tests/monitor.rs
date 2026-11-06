// SPDX-License-Identifier: MIT

use super::*;

/// What a monitor change reports: the splices and the cursor it leaves.
type Applied = (Vec<EntrySplice>, Option<usize>);

fn watching(names: &[&str]) -> NavigationState {
    loaded("/home", children("/home", names))
}

fn change_home(state: &mut NavigationState, change: DirectoryChange) -> Option<Applied> {
    state.apply_directory_change(0, &location("/home"), change)
}

/// Each splice's position and the number of rows it removed.
fn splice_spans(splices: &[EntrySplice]) -> Vec<(usize, usize)> {
    splices
        .iter()
        .map(|splice| (splice.position, splice.removed))
        .collect()
}

fn png_with_details(path: &str, name: &str) -> FileEntry {
    FileEntry {
        size: MetadataValue::Known(100),
        modified_unix_seconds: MetadataValue::Known(10),
        created_unix_seconds: MetadataValue::Unknown,
        ..named_entry(path, name)
    }
}

/// The renamed `new.png` a monitor reports, altered as `change` names.
fn renamed_png(change: &str) -> FileEntry {
    let mut renamed = png_with_details("/home/new.png", "new.png");
    match change {
        "size" => renamed.size = MetadataValue::Known(200),
        "mtime" => renamed.modified_unix_seconds = MetadataValue::Known(11),
        "unknown" => renamed.modified_unix_seconds = MetadataValue::Unknown,
        "extension" => {
            renamed.native_name = "new.mp4".into();
            renamed.display_name = "new.mp4".into();
            renamed.location = location("/home/new.mp4");
        }
        "fresh details" => renamed.image_dimensions = MetadataValue::Known((40, 50)),
        _ => {}
    }
    renamed
}

#[test]
fn camera_device_order_monitor_changes_preserve_existing_positions() {
    let mut state = NavigationState::default();
    let watched = Location::uri("gphoto2://camera/");
    state.navigate(watched.clone(), RequestId(1));
    let photo = |name: &str| FileEntry {
        location: Location::uri(format!("gphoto2://camera/202609_a/{name}")),
        kind: EntryKind::File,
        ..named_entry("/unused", name)
    };
    state.apply_batch(RequestId(1), vec![photo("z.jpg"), photo("m.jpg")]);
    state.set_selection(0, &[1], Some(1));
    let (splices, _) = state
        .apply_directory_change(0, &watched, DirectoryChange::Upsert(photo("a.jpg")))
        .expect("new photo appended");
    assert_eq!(splices[0].position, 2);
    let updated = FileEntry {
        display_name: "b.jpg".into(),
        size: MetadataValue::Known(99),
        ..photo("m.jpg")
    };
    let (splices, _) = state
        .apply_directory_change(0, &watched, DirectoryChange::Upsert(updated))
        .expect("existing photo updated");
    assert_eq!(splice_spans(&splices), [(1, 1)]);
    assert_eq!(state.selected_positions(0), [1]);
    assert_eq!(column_names(&state, 0), ["z.jpg", "b.jpg", "a.jpg"]);
    state.apply_directory_change(
        0,
        &watched,
        DirectoryChange::Remove(photo("z.jpg").location),
    );
    assert_eq!(state.selected_positions(0), [0]);
    assert_eq!(state.columns[0].entries[0].size, MetadataValue::Known(99));
}

#[test]
fn monitor_insertions_follow_the_active_sort_order() {
    let mut state = watching(&["alpha", "charlie"]);

    let (splices, _) = change_home(
        &mut state,
        DirectoryChange::Upsert(named_entry("/home/bravo", "bravo")),
    )
    .expect("the new entry should change the column");

    assert_eq!(splice_spans(&splices), [(1, 0)]);
    assert_eq!(splices[0].entries[0].display_name, "bravo");
}

#[test]
fn monitor_updates_reposition_only_the_changed_entry() {
    let mut state = watching(&ABC);

    let (splices, _) = change_home(
        &mut state,
        DirectoryChange::Upsert(named_entry("/home/alpha", "zulu")),
    )
    .expect("renaming an entry should change the column");

    assert_eq!(
        (splices.len(), splices[0].removed, splices[1].entries.len()),
        (2, 1, 1)
    );
    assert_eq!(state.columns[0].entries[2].display_name, "zulu");
}

#[test]
fn monitor_updates_to_existing_entry_at_same_position_uses_single_splice() {
    let mut state = watching(&ABC);
    let updated = FileEntry {
        size: MetadataValue::Known(9999),
        ..named_entry("/home/bravo", "bravo")
    };

    let (splices, _) = change_home(&mut state, DirectoryChange::Upsert(updated))
        .expect("updating an existing entry's metadata should change the column");

    assert_eq!(splice_spans(&splices), [(1, 1)]);
    let inserted: Vec<_> = splices[0].entries.iter().map(|entry| &entry.size).collect();
    assert_eq!(inserted, [&MetadataValue::Known(9999)]);
    assert_eq!(state.columns[0].entries[1].size, MetadataValue::Known(9999));
}

#[test]
fn monitor_removals_preserve_selection_by_native_location() {
    let mut state = watching(&["alpha", "bravo"]);
    assert!(state.select(0, 1));

    let (_, selected) = change_home(&mut state, DirectoryChange::Remove(location("/home/alpha")))
        .expect("removing an entry should change the column");

    assert_eq!(selected, Some(0));
    assert_eq!(state.columns[0].entries[0].display_name, "bravo");
}

#[test]
fn removing_the_selected_entry_focuses_its_nearest_neighbor() {
    for preserve_fill in [false, true] {
        let mut state = NavigationState::default();
        state.set_preserve_fill_on_removal(preserve_fill);
        list(&mut state, "/home", children("/home", &ABC));
        assert!(state.select(0, 1));

        let (_, selected) =
            change_home(&mut state, DirectoryChange::Remove(location("/home/bravo")))
                .expect("removing the selected entry should change the column");

        assert_eq!(selected, Some(1));
        assert_eq!(state.columns[0].entries[1].display_name, "charlie");
        assert_eq!(
            state.columns[0]
                .selected_locations
                .contains(&location("/home/charlie")),
            !preserve_fill
        );
    }
}

#[test]
fn monitor_moves_follow_the_selected_entry() {
    let mut state = watching(&["old"]);
    assert!(state.select(0, 0));

    let (_, selected) = change_home(
        &mut state,
        DirectoryChange::Move {
            from: location("/home/old"),
            entry: named_entry("/home/new", "new"),
        },
    )
    .expect("moving an entry should change the column");

    assert_eq!(selected, Some(0));
    assert_eq!(state.columns[0].entries[0].location, location("/home/new"));
}

#[test]
fn monitor_moves_preserve_loaded_details_only_for_unchanged_files() {
    for change in [
        "name",
        "size",
        "mtime",
        "unknown",
        "extension",
        "fresh details",
    ] {
        let old = FileEntry {
            image_dimensions: MetadataValue::Known((20, 30)),
            duration_seconds: MetadataValue::Unavailable,
            child_count: MetadataValue::Unavailable,
            ..png_with_details("/home/old.png", "old.png")
        };
        let mut state = loaded("/home", vec![old]);
        let (splices, _) = change_home(
            &mut state,
            DirectoryChange::Move {
                from: location("/home/old.png"),
                entry: renamed_png(change),
            },
        )
        .expect("move");
        let expected = match change {
            "name" => MetadataValue::Known((20, 30)),
            "fresh details" => MetadataValue::Known((40, 50)),
            _ => MetadataValue::Unknown,
        };
        let entry = &state.columns[0].entries[0];
        assert_eq!(entry.image_dimensions, expected, "{change}");
        assert_eq!(
            splices.last().expect("insertion").entries[0].image_dimensions,
            expected,
            "{change}"
        );
        if change == "name" {
            assert_eq!(entry.duration_seconds, MetadataValue::Unavailable);
            assert_eq!(entry.child_count, MetadataValue::Unavailable);
        }
    }
}

#[test]
fn external_moves_rebase_open_descendant_locations() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    assert!(state.descend(0, location("/home/old"), RequestId(2)));
    assert!(state.descend(1, location("/home/old/deep"), RequestId(3)));

    let path = state
        .path_after_external_change(
            0,
            &DirectoryChange::Move {
                from: location("/home/old"),
                entry: named_entry("/home/new", "new"),
            },
        )
        .expect("the open path should be rebased");

    assert_eq!(
        path.locations(),
        &[
            location("/home"),
            location("/home/new"),
            location("/home/new/deep"),
        ]
    );
}

#[test]
fn external_removals_close_affected_descendant_columns() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    assert!(state.descend(0, location("/home/removed"), RequestId(2)));

    let path = state
        .path_after_external_change(0, &DirectoryChange::Remove(location("/home/removed")))
        .expect("the removed open path should be closed");

    assert_eq!(path.locations(), &[location("/home")]);
}

#[test]
fn remote_external_removals_close_the_exact_open_column() {
    let root = Location::uri("sftp://user@host/mnt/share");
    let removed = Location::uri("sftp://user@host/mnt/share/removed");
    let mut state = NavigationState::default();
    state.navigate(root.clone(), RequestId(1));
    assert!(state.descend(0, removed.clone(), RequestId(2)));

    let path = state
        .path_after_external_change(0, &DirectoryChange::Remove(removed))
        .expect("the removed remote column should be closed");

    assert_eq!(path.locations(), &[root]);
}
