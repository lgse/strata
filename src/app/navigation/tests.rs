// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsString;

use super::*;
use crate::{
    model::{EntryKind, MetadataValue},
    services::DirectoryChange,
};

fn location(path: &str) -> Location {
    Location::local(path)
}

fn entry(path: &str) -> FileEntry {
    named_entry(path, "child")
}

fn named_entry(path: &str, name: &str) -> FileEntry {
    FileEntry {
        location: location(path),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
    }
}

#[test]
fn multi_selection_tracks_entries_and_replaces_cleanly() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/alpha", "alpha"),
            named_entry("/fixture/bravo", "bravo"),
            named_entry("/fixture/charlie", "charlie"),
        ],
    );

    assert!(state.set_selection(0, &[0, 2], Some(2)));
    assert_eq!(
        state
            .selected_entries()
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "charlie"]
    );
    assert_eq!(
        state.focused_entry().map(|(_, position, _)| position),
        Some(2)
    );

    assert!(state.select(0, 1));
    assert_eq!(
        state
            .selected_entries()
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["bravo"]
    );
}

#[test]
fn keyboard_range_selection_extends_and_contracts_from_its_anchor() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/alpha", "alpha"),
            named_entry("/fixture/bravo", "bravo"),
            named_entry("/fixture/charlie", "charlie"),
        ],
    );
    assert!(state.select(0, 0));

    assert_eq!(
        state.extend_selection(1).map(|(_, _, range)| range),
        Some(vec![0, 1])
    );
    assert_eq!(
        state.extend_selection(1).map(|(_, _, range)| range),
        Some(vec![0, 1, 2])
    );
    assert_eq!(
        state.extend_selection(-1).map(|(_, _, range)| range),
        Some(vec![0, 1])
    );
    assert_eq!(state.selected_entries().len(), 2);
}

#[test]
fn active_path_is_independent_from_the_parent_highlight() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/active", "active"),
            named_entry("/fixture/hovered", "hovered"),
        ],
    );
    assert!(state.select(0, 0));
    assert!(state.descend(0, location("/fixture/active"), RequestId(2)));

    assert!(state.select(0, 1));

    assert_eq!(state.active_child_position(0), Some(0));
    assert_eq!(state.columns[0].selected, Some(1));
}

#[test]
fn monitor_insertions_follow_the_active_sort_order() {
    let mut state = NavigationState::default();
    let watched = location("/home");
    state.navigate(watched.clone(), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/home/alpha", "alpha"),
            named_entry("/home/charlie", "charlie"),
        ],
    );

    let (splices, _) = state
        .apply_directory_change(
            0,
            &watched,
            DirectoryChange::Upsert(named_entry("/home/bravo", "bravo")),
        )
        .expect("the new entry should change the column");

    assert_eq!(splices.len(), 1);
    assert_eq!(splices[0].position, 1);
    assert_eq!(splices[0].removed, 0);
    assert_eq!(splices[0].entries[0].display_name, "bravo");
}

#[test]
fn monitor_updates_reposition_only_the_changed_entry() {
    let mut state = NavigationState::default();
    let watched = location("/home");
    state.navigate(watched.clone(), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/home/alpha", "alpha"),
            named_entry("/home/bravo", "bravo"),
            named_entry("/home/charlie", "charlie"),
        ],
    );

    let (splices, _) = state
        .apply_directory_change(
            0,
            &watched,
            DirectoryChange::Upsert(named_entry("/home/alpha", "zulu")),
        )
        .expect("renaming an entry should change the column");

    assert_eq!(splices.len(), 2);
    assert_eq!(splices[0].removed, 1);
    assert_eq!(splices[1].entries.len(), 1);
    assert_eq!(state.columns[0].entries[2].display_name, "zulu");
}

#[test]
fn monitor_removals_preserve_selection_by_native_location() {
    let mut state = NavigationState::default();
    let watched = location("/home");
    state.navigate(watched.clone(), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/home/alpha", "alpha"),
            named_entry("/home/bravo", "bravo"),
        ],
    );
    assert!(state.select(0, 1));

    let (_, selected) = state
        .apply_directory_change(
            0,
            &watched,
            DirectoryChange::Remove(location("/home/alpha")),
        )
        .expect("removing an entry should change the column");

    assert_eq!(selected, Some(0));
    assert_eq!(state.columns[0].entries[0].display_name, "bravo");
}

#[test]
fn removing_the_selected_entry_focuses_its_nearest_neighbor() {
    let mut state = NavigationState::default();
    let watched = location("/home");
    state.navigate(watched.clone(), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/home/alpha", "alpha"),
            named_entry("/home/bravo", "bravo"),
            named_entry("/home/charlie", "charlie"),
        ],
    );
    assert!(state.select(0, 1));

    let (_, selected) = state
        .apply_directory_change(
            0,
            &watched,
            DirectoryChange::Remove(location("/home/bravo")),
        )
        .expect("removing the selected entry should change the column");

    assert_eq!(selected, Some(1));
    assert_eq!(state.columns[0].entries[1].display_name, "charlie");
}

#[test]
fn monitor_moves_follow_the_selected_entry() {
    let mut state = NavigationState::default();
    let watched = location("/home");
    state.navigate(watched.clone(), RequestId(1));
    state.apply_batch(RequestId(1), vec![named_entry("/home/old", "old")]);
    assert!(state.select(0, 0));

    let (_, selected) = state
        .apply_directory_change(
            0,
            &watched,
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
fn selecting_a_sibling_replaces_deeper_columns() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    assert!(state.descend(0, location("/home/one"), RequestId(2)));
    assert!(state.descend(1, location("/home/one/deep"), RequestId(3)));

    assert!(state.descend(0, location("/home/two"), RequestId(4)));

    assert_eq!(state.columns.len(), 2);
    assert_eq!(state.columns[1].location, location("/home/two"));
}

#[test]
fn stale_batches_are_rejected() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.navigate(location("/tmp"), RequestId(2));

    assert!(
        state
            .apply_batch(RequestId(1), vec![entry("/home/child")])
            .is_none()
    );
    assert!(state.columns[0].entries.is_empty());
}

#[test]
fn empty_is_distinct_from_loading_and_error() {
    let mut state = NavigationState::default();
    state.navigate(location("/empty"), RequestId(1));
    assert_eq!(state.columns[0].load_state, LoadState::Loading);

    assert_eq!(state.finish(RequestId(1), false, None), Some(0));
    assert_eq!(state.columns[0].load_state, LoadState::Empty);
}

#[test]
fn truncated_load_state_survives_until_reload() {
    let mut state = NavigationState::default();
    state.navigate(location("/partial"), RequestId(1));

    assert_eq!(state.finish(RequestId(1), true, None), Some(0));
    assert!(state.columns[0].truncated);

    state.reload_column(0, RequestId(2));
    assert!(!state.columns[0].truncated);
}

#[test]
fn navigation_availability_tracks_history_and_parent() {
    let mut state = NavigationState::default();
    assert!(!state.can_go_back());
    assert!(!state.can_go_forward());
    assert!(!state.can_go_parent());

    state.navigate(location("/"), RequestId(1));
    assert!(!state.can_go_parent());
    state.navigate(location("/home"), RequestId(2));
    assert!(state.can_go_back());
    assert!(state.can_go_parent());
    assert!(!state.can_go_forward());

    let back = state.go_back().expect("back history should be available");
    state.restore(back, [RequestId(3)]);
    assert!(!state.can_go_back());
    assert!(state.can_go_forward());
}

#[test]
fn back_and_forward_restore_committed_paths() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    assert!(state.descend(0, location("/home/projects"), RequestId(2)));

    let back = state
        .go_back()
        .expect("a committed path should be available");
    assert_eq!(back.locations(), &[location("/home")]);
    state.restore(back, [RequestId(3)]);

    let forward = state
        .go_forward()
        .expect("the descended path should be available");
    assert_eq!(
        forward.locations(),
        &[location("/home"), location("/home/projects")]
    );
}

#[test]
fn parent_removes_the_deepest_committed_column() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    assert!(state.descend(0, location("/home/projects"), RequestId(2)));

    let parent = state.go_parent().expect("the path has a parent");
    assert_eq!(parent.locations(), &[location("/home")]);
}

fn hidden_entry(path: &str, name: &str) -> FileEntry {
    FileEntry {
        location: location(path),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind: EntryKind::Directory,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        is_hidden: true,
        mode: MetadataValue::Unknown,
    }
}

#[test]
fn keyboard_selection_skips_hidden_entries_when_hidden_files_are_not_shown() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/home/alpha", "alpha"),
            hidden_entry("/home/bravo", "bravo"),
            named_entry("/home/charlie", "charlie"),
        ],
    );

    assert_eq!(state.move_selection(1), Some((0, 0)));
    assert_eq!(state.move_selection(1), Some((0, 2)));
    assert_eq!(state.move_selection(1), Some((0, 2)));
    assert_eq!(state.move_selection(-1), Some((0, 0)));
    assert_eq!(state.move_selection(-1), Some((0, 0)));

    state.set_show_hidden(true);
    assert_eq!(state.move_selection(1), Some((0, 1)));
    assert_eq!(state.move_selection(1), Some((0, 2)));
}

#[test]
fn staged_keyboard_descent_selects_the_first_visible_entry() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.select_first_on_load(0);
    state.install_snapshot(
        RequestId(1),
        vec![
            hidden_entry("/home/.hidden", ".hidden"),
            named_entry("/home/visible", "visible"),
        ],
    );

    assert_eq!(
        state.focused_entry().map(|(_, position, _)| position),
        Some(1)
    );
}

#[test]
fn keyboard_selection_extension_skips_hidden_entries() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/home/alpha", "alpha"),
            hidden_entry("/home/bravo", "bravo"),
            named_entry("/home/charlie", "charlie"),
        ],
    );
    assert!(state.select(0, 0));

    assert_eq!(
        state.extend_selection(1).map(|(_, _, range)| range),
        Some(vec![0, 2])
    );
    assert_eq!(state.selected_entries().len(), 2);
    assert!(!state.selected_entries().iter().any(|e| e.is_hidden));
}

#[test]
fn keyboard_selection_is_bounded_and_tracks_the_active_column() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.apply_batch(RequestId(1), vec![entry("/home/one"), entry("/home/two")]);

    assert_eq!(state.move_selection(1), Some((0, 0)));
    assert_eq!(state.move_selection(1), Some((0, 1)));
    assert_eq!(state.move_selection(1), Some((0, 1)));
    assert_eq!(state.move_selection(-1), Some((0, 0)));
    assert_eq!(state.move_selection(-1), Some((0, 0)));
}

#[test]
fn moving_between_parent_and_child_columns_restores_their_selections() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.apply_batch(RequestId(1), vec![entry("/home/projects")]);
    assert!(state.select(0, 0));
    assert!(state.descend(0, location("/home/projects"), RequestId(2)));
    state.select_first_on_load(1);
    state.apply_batch(RequestId(2), vec![entry("/home/projects/strata")]);

    assert_eq!(state.focus_parent(), Some((0, Some(0))));
    let (depth, position, focused) = state.focused_entry().expect("parent entry remains focused");
    assert_eq!((depth, position), (0, 0));
    assert_eq!(focused.location, location("/home/projects"));

    assert_eq!(state.focus_child(), Some((1, Some(0))));
    let (depth, position, focused) = state.focused_entry().expect("child entry remains focused");
    assert_eq!((depth, position), (1, 0));
    assert_eq!(focused.location, location("/home/projects/strata"));
    assert_eq!(state.focus_child(), None);
}

#[test]
fn closing_the_deepest_column_preserves_the_parent_selection() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.apply_batch(RequestId(1), vec![entry("/home/projects")]);
    assert!(state.select(0, 0));
    assert!(state.descend(0, location("/home/projects"), RequestId(2)));

    assert_eq!(state.close_deepest(), Some((0, Some(0))));
    assert_eq!(state.columns.len(), 1);
    assert_eq!(state.close_deepest(), None);
}

#[test]
fn closing_a_middle_column_removes_it_and_its_descendants() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    state.apply_batch(RequestId(1), vec![entry("/home/projects")]);
    assert!(state.select(0, 0));
    assert!(state.descend(0, location("/home/projects"), RequestId(2)));
    state.apply_batch(RequestId(2), vec![entry("/home/projects/strata")]);
    assert!(state.select(1, 0));
    assert!(state.descend(1, location("/home/projects/strata"), RequestId(3)));

    assert_eq!(state.close_from(1), Some((0, Some(0))));
    assert_eq!(state.columns.len(), 1);
    assert_eq!(state.close_from(0), None);
}

#[test]
fn batches_are_merged_into_one_global_sort_order() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(RequestId(1), vec![named_entry("/fixture/z", "z")]);
    assert!(state.select(0, 0));

    let (_, insertions) = state
        .apply_batch(RequestId(1), vec![named_entry("/fixture/a", "a")])
        .expect("the request is current");

    assert_eq!(insertions.len(), 1);
    assert_eq!(insertions[0].position, 0);
    assert_eq!(state.columns[0].entries[0].display_name, "a");
    assert_eq!(state.columns[0].entries[1].display_name, "z");
    assert_eq!(state.columns[0].selected, Some(1));
}

#[test]
fn names_are_sorted_case_insensitively() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/apple", "apple"),
            named_entry("/fixture/Banana", "Banana"),
            named_entry("/fixture/cherry", "cherry"),
            named_entry("/fixture/Date", "Date"),
        ],
    );

    assert_eq!(
        state.columns[0]
            .entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["apple", "Banana", "cherry", "Date"]
    );
}

#[test]
fn names_that_differ_only_by_case_have_a_deterministic_order() {
    assert_eq!(compare_display_names("file", "FILE"), Ordering::Greater);
    assert_eq!(
        compare_display_names("Straße", "STRASSE"),
        Ordering::Greater
    );
}

#[test]
fn changing_sort_preferences_preserves_the_selected_entry() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/a", "a"),
            named_entry("/fixture/z", "z"),
        ],
    );
    assert!(state.select(0, 0));

    assert!(
        state
            .apply_sort_preferences(
                0,
                ViewPreferences {
                    sort_direction: SortDirection::Descending,
                    ..ViewPreferences::default()
                },
            )
            .is_some()
    );

    assert_eq!(state.columns[0].entries[0].display_name, "z");
    assert_eq!(state.columns[0].selected, Some(1));
    assert_eq!(
        state
            .focused_entry()
            .map(|(_, _, entry)| entry.display_name),
        Some("a".into())
    );
}

#[test]
fn changed_sort_preferences_are_inherited_by_new_columns() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    let preferences = ViewPreferences {
        folders_first: false,
        sort_key: SortKey::Modified,
        sort_direction: SortDirection::Descending,
        ..ViewPreferences::default()
    };

    assert!(state.apply_sort_preferences(0, preferences).is_some());
    assert!(state.descend(0, location("/fixture/child"), RequestId(2)));

    assert_eq!(state.column_preferences(1), Some(preferences));
}

#[test]
fn changing_sort_preferences_only_reorders_the_target_column() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/a", "a"),
            named_entry("/fixture/z", "z"),
        ],
    );
    assert!(state.descend(0, location("/fixture/child"), RequestId(2)));
    state.apply_batch(
        RequestId(2),
        vec![
            named_entry("/fixture/child/a", "a"),
            named_entry("/fixture/child/z", "z"),
        ],
    );

    assert!(
        state
            .apply_sort_preferences(
                1,
                ViewPreferences {
                    sort_direction: SortDirection::Descending,
                    ..ViewPreferences::default()
                },
            )
            .is_some()
    );

    assert_eq!(state.columns[0].entries[0].display_name, "a");
    assert_eq!(state.columns[1].entries[0].display_name, "z");
}

fn file_entry(path: &str, name: &str) -> FileEntry {
    FileEntry {
        location: location(path),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

fn metadata_update(path: &str, size: u64, modified: i64) -> MetadataUpdate {
    MetadataUpdate {
        location: location(path),
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Known(modified),
        mode: MetadataValue::Unknown,
    }
}

#[test]
fn metadata_fill_updates_values_without_reordering() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            file_entry("/fixture/bravo", "bravo"),
            file_entry("/fixture/alpha", "alpha"),
        ],
    );
    let order_before: Vec<_> = state.columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();

    let applied = state.apply_metadata(
        RequestId(1),
        vec![
            metadata_update("/fixture/alpha", 10, 100),
            metadata_update("/fixture/bravo", 20, 200),
        ],
    );

    assert_eq!(applied, Some((0, vec![0, 1])));
    let entries = &state.columns[0].entries;
    assert_eq!(entries[0].size, MetadataValue::Known(10));
    assert_eq!(entries[1].modified_unix_seconds, MetadataValue::Known(200));
    let order_after: Vec<_> = entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(order_before, order_after);
}

#[test]
fn metadata_fill_for_a_superseded_load_is_dropped() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(RequestId(1), vec![file_entry("/fixture/alpha", "alpha")]);

    assert_eq!(
        state.apply_metadata(
            RequestId(2),
            vec![metadata_update("/fixture/alpha", 10, 100)]
        ),
        None
    );
    assert_eq!(
        state.apply_metadata(
            RequestId(1),
            vec![metadata_update("/fixture/ghost", 10, 100)]
        ),
        None
    );
    assert_eq!(state.columns[0].entries[0].size, MetadataValue::Unknown);
}

#[test]
fn depth_for_request_tracks_live_loads_only() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    assert_eq!(state.depth_for_request(RequestId(1)), Some(0));
    assert_eq!(state.depth_for_request(RequestId(9)), None);

    assert!(state.descend(0, location("/fixture/sub"), RequestId(2)));
    assert_eq!(state.depth_for_request(RequestId(2)), Some(1));

    state.reload_column(0, RequestId(3));
    assert_eq!(state.depth_for_request(RequestId(1)), None);
    assert_eq!(state.depth_for_request(RequestId(3)), Some(0));
}

#[test]
fn positioned_fill_applies_in_place_and_flags_moved_rows() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            file_entry("/fixture/alpha", "alpha"),
            file_entry("/fixture/bravo", "bravo"),
            file_entry("/fixture/charlie", "charlie"),
        ],
    );

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
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    let mut half_known = file_entry("/fixture/half", "half");
    half_known.size = MetadataValue::Known(50);
    state.apply_batch(RequestId(1), vec![half_known]);

    let applied = state.apply_metadata(
        RequestId(1),
        vec![MetadataUpdate {
            location: location("/fixture/half"),
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Known(300),
            mode: MetadataValue::Known(0o100640),
        }],
    );
    assert!(matches!(applied, Some((0, ref positions)) if positions == &[0]));
    assert_eq!(state.columns[0].entries[0].size, MetadataValue::Known(50));
    assert_eq!(
        state.columns[0].entries[0].modified_unix_seconds,
        MetadataValue::Known(300)
    );
    assert_eq!(
        state.columns[0].entries[0].mode,
        MetadataValue::Known(0o100640)
    );

    let applied = state.apply_metadata(
        RequestId(1),
        vec![MetadataUpdate {
            location: location("/fixture/half"),
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
        }],
    );
    assert_eq!(applied, None);
}

#[test]
fn gap_targets_cover_directory_mtimes_but_never_directory_sizes() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    let mut complete_file = file_entry("/fixture/done", "done");
    complete_file.size = MetadataValue::Known(5);
    complete_file.modified_unix_seconds = MetadataValue::Known(60);
    let mut half_file = file_entry("/fixture/half", "half");
    half_file.size = MetadataValue::Known(50);
    state.apply_batch(
        RequestId(1),
        vec![named_entry("/fixture/sub", "sub"), complete_file, half_file],
    );

    assert_eq!(
        state.column_unknown_metadata(0).map(|targets| {
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
fn selected_count_reports_without_cloning_entries() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    state.apply_batch(
        RequestId(1),
        vec![
            named_entry("/fixture/alpha", "alpha"),
            named_entry("/fixture/bravo", "bravo"),
            named_entry("/fixture/charlie", "charlie"),
        ],
    );
    assert_eq!(state.selected_count(), 0);
    assert!(state.set_selection(0, &[0, 2], Some(2)));
    assert_eq!(state.selected_count(), 2);
}
