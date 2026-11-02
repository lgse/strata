// SPDX-License-Identifier: MIT

use super::*;

fn focus_target(state: &NavigationState) -> Option<(usize, usize, Location)> {
    state
        .focused_entry()
        .map(|(depth, position, entry)| (depth, position, entry.location))
}

/// `/home` with `projects` selected and opened as the second column.
fn projects_open() -> NavigationState {
    let mut state = loaded("/home", vec![entry("/home/projects")]);
    open_first(&mut state, 0, "/home/projects", 2);
    state
}

#[test]
fn shared_defaults_do_not_replace_existing_column_sort_selection_or_location() {
    let mut state = loaded("/fixture", vec![entry("/fixture/child")]);
    state.set_selection(0, &[0], Some(0));
    let local = state.column_preferences(0).expect("existing column");
    let path = state.current_path();
    let defaults = ViewPreferences {
        sort_key: SortKey::Size,
        sort_direction: SortDirection::Descending,
        ..local
    };
    state.set_default_preferences(defaults);
    assert_eq!(state.column_preferences(0), Some(local));
    assert_eq!(state.current_path(), path);
    assert_eq!(state.selected_positions(0), [0]);
    state.descend(0, location("/fixture/child"), RequestId(2));
    assert_eq!(state.column_preferences(1), Some(defaults));
}

#[test]
fn focusing_a_column_preserves_selection_and_descendants() {
    let mut state = loaded("/fixture", children("/fixture", &["alpha", "bravo"]));
    state.set_selection(0, &[0, 1], Some(1));
    state.descend(0, location("/fixture/alpha"), RequestId(2));
    let path = state.current_path();
    assert!(state.focus_column(0));
    assert_eq!(fill_and_focus(&state, 0), (vec![0, 1], Some((0, Some(1)))));
    assert_eq!(state.current_path(), path);

    assert!(state.focus_column(1));
    let child_focus = (state.active_focus(), state.selected_entries());
    assert_eq!(child_focus, (Some((1, None)), Vec::new()));
    let missing_column_focused = state.focus_column(2);
    assert_eq!(
        (missing_column_focused, state.active_depth()),
        (false, Some(1))
    );
}

#[test]
fn active_path_is_independent_from_the_parent_highlight() {
    let mut state = loaded("/fixture", children("/fixture", &["active", "hovered"]));
    open_first(&mut state, 0, "/fixture/active", 2);

    assert!(state.select(0, 1));

    assert_eq!(
        (state.active_child_position(0), state.columns[0].selected),
        (Some(0), Some(1))
    );
}

#[test]
fn relocating_a_column_preserves_selection_preferences_and_active_depth() {
    let mut state = loaded("/home", vec![named_entry("/home/old", "old")]);
    state.select(0, 0);
    state.descend(0, location("/home/old"), RequestId(2));
    state.apply_batch(RequestId(2), children("/home/old", &["one", "two"]));
    state.set_selection(1, &[0, 1], Some(1));
    state.apply_sort_preferences(1, descending());
    state.focus_column(0);

    state.relocate_column(1, location("/home/renamed"), RequestId(3));
    assert_eq!(
        (
            state.active_depth(),
            state.selected_positions(0),
            state.column_preferences(1)
        ),
        (Some(0), vec![0], Some(descending()))
    );
    assert!(
        state
            .apply_batch(RequestId(2), vec![named_entry("/home/old/stale", "stale")])
            .is_none()
    );
    state.apply_batch(RequestId(3), children("/home/renamed", &["one", "two"]));
    assert_eq!(state.selected_positions(1), [0, 1]);
    let column = &state.columns[1];
    assert_eq!(
        column.entries[column.selected.expect("keyboard cursor")].display_name,
        "two"
    );
    assert!(
        column
            .selection_anchor
            .as_ref()
            .expect("selection anchor")
            .is_within(&location("/home/renamed"))
    );
}

#[test]
fn selecting_a_sibling_replaces_deeper_columns() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    for (depth, path, request) in [(0, "/home/one", 2), (1, "/home/one/deep", 3)] {
        assert!(state.descend(depth, location(path), RequestId(request)));
    }

    assert!(state.descend(0, location("/home/two"), RequestId(4)));

    assert_eq!(
        (state.columns.len(), &state.columns[1].location),
        (2, &location("/home/two"))
    );
}

#[test]
fn moving_between_parent_and_child_columns_restores_their_selections() {
    let mut state = projects_open();
    state.select_first_on_load(1);
    state.apply_batch(RequestId(2), vec![entry("/home/projects/strata")]);

    assert_eq!(state.focus_parent(), Some((0, Some(0))));
    assert_eq!(
        focus_target(&state),
        Some((0, 0, location("/home/projects"))),
        "the parent entry remains focused"
    );

    assert_eq!(state.focus_child(), Some((1, Some(0))));
    assert_eq!(
        focus_target(&state),
        Some((1, 0, location("/home/projects/strata"))),
        "the child entry remains focused"
    );
    assert_eq!(state.focus_child(), None);
}

#[test]
fn dragging_an_open_parent_folder_does_not_claim_the_child_selection() {
    let mut state = projects_open();
    state.apply_batch(RequestId(2), vec![entry("/home/projects/strata")]);
    assert!(state.select(1, 0));

    assert!(!state.active_selection_contains(0, 0));
    assert!(state.active_selection_contains(1, 0));
}

#[test]
fn closing_the_deepest_column_preserves_the_parent_selection() {
    let mut state = projects_open();

    assert_eq!(state.close_deepest(), Some((0, Some(0))));
    assert_eq!(state.columns.len(), 1);
    assert_eq!(state.close_deepest(), None);
}

#[test]
fn closing_a_middle_column_removes_it_and_its_descendants() {
    let mut state = projects_open();
    state.apply_batch(RequestId(2), vec![entry("/home/projects/strata")]);
    open_first(&mut state, 1, "/home/projects/strata", 3);

    assert_eq!(state.close_from(1), Some((0, Some(0))));
    assert_eq!(state.columns.len(), 1);
    assert_eq!(state.close_from(0), None);
}
