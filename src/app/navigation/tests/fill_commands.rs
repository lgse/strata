// SPDX-License-Identifier: MIT

use super::*;

/// Toggles the cursor's row and returns the fill of the column at `depth`.
fn toggle(state: &mut NavigationState, depth: usize) -> (CursorToggle, Vec<usize>) {
    let toggled = state.toggle_cursor_fill();
    (toggled, state.selected_positions(depth))
}

/// Moves the cursor `count` rows on and returns where it lands with the root fill.
fn page(state: &mut NavigationState, count: usize) -> (Option<usize>, Vec<usize>) {
    let position = state
        .page_cursor(1, count, None)
        .map(|(_, position, _)| position);
    (position, state.selected_positions(0))
}

fn fill_and_cursor(state: &NavigationState) -> (Vec<usize>, Option<usize>) {
    (state.selected_positions(0), focused_position(state))
}

fn sorted_names(state: &NavigationState, positions: &[usize]) -> Vec<String> {
    let mut selected: Vec<_> = positions
        .iter()
        .map(|position| state.columns[0].entries[*position].display_name.clone())
        .collect();
    selected.sort();
    selected
}

#[test]
fn space_adds_a_load_cursor_then_moves_without_rewriting_the_fill() {
    let mut state = listing_with_the_first_entry_selected();
    assert_eq!(
        (
            state.selection_is_load_cursor(),
            state.selected_positions(0)
        ),
        (true, vec![0])
    );

    assert_eq!(state.toggle_cursor_fill(), CursorToggle::Added);
    assert_eq!(
        (state.selection_is_load_cursor(), fill_and_cursor(&state)),
        (false, (vec![0], Some(0)))
    );

    let (depth, position, cleared) = state.page_cursor(1, 1, None).expect("next item");
    assert_eq!((depth, position), (0, 1));
    assert!(!cleared, "leaving a committed fill does not clear it");
    assert_eq!(fill_and_cursor(&state), (vec![0], Some(1)));
    let selected = selected_names(&state);
    assert!(selected.iter().all(|name| name == "alpha"));
    assert_eq!(focused_name(&state), Some("bravo".to_owned()));

    for (fill, reason) in [
        (vec![0, 1], "the cursor moves on past the new fill"),
        (
            vec![0, 1, 2],
            "the last item does not move past the listing",
        ),
    ] {
        assert_eq!(
            toggle(&mut state, 0),
            (CursorToggle::Added, fill.clone()),
            "{reason}"
        );
        assert_eq!(page(&mut state, 1), (Some(2), fill), "{reason}");
    }
    assert_eq!(toggle(&mut state, 0), (CursorToggle::Removed, vec![0, 1]));
    assert_eq!(focused_position(&state), Some(2));
    assert!(selected_names(&state).iter().all(|name| name != "charlie"));
}

#[test]
fn compact_select_all_excludes_entries_that_arrive_later() {
    let mut state = listing_without_a_load_cursor();

    assert_eq!(state.select_all(0), Some(2));
    assert_eq!(state.selected_count(), 3);
    state.apply_batch(
        RequestId(1),
        vec![named_entry("/fixture/aardvark", "aardvark")],
    );

    assert_eq!(
        (state.selected_count(), state.selected_positions(0)),
        (3, vec![1, 2, 3])
    );
    assert_eq!(toggle(&mut state, 0), (CursorToggle::Removed, vec![1, 2]));
}

#[test]
fn a_drag_over_select_all_keeps_later_entries_unselected() {
    let mut state = listing_without_a_load_cursor();

    assert_eq!(state.select_all(0), Some(2));
    state.commit_selection();
    assert!(state.set_selection(0, &[0, 2], Some(2)));
    assert_eq!(selected_names(&state), ["alpha", "charlie"]);
    state.apply_batch(
        RequestId(1),
        vec![named_entry("/fixture/aardvark", "aardvark")],
    );

    assert_eq!(state.selected_count(), 2);
    assert_eq!(state.selected_positions(0), [1, 3]);
    assert_eq!(selected_names(&state), ["alpha", "charlie"]);
    // An atomic save renames a file the listing never showed.
    state.apply_directory_change(
        0,
        &location("/fixture"),
        DirectoryChange::Move {
            from: location("/fixture/delta.tmp"),
            entry: named_entry("/fixture/delta", "delta"),
        },
    );
    assert_eq!(selected_names(&state), ["alpha", "charlie"]);
}

#[test]
fn select_all_and_invert_keep_the_cursor_and_other_columns() {
    let mut state = listing_with_the_first_entry_selected();
    state.descend(0, location("/fixture/alpha"), RequestId(2));
    state.select_first_on_load(1);
    state.apply_batch(RequestId(2), children("/fixture/alpha", &["one", "two"]));
    state.focus_column(0);
    let selected = state.select_visible(0).expect("parent entries");
    assert_eq!(selected, (0, vec![0, 1, 2]));
    assert_eq!(
        state.selected_positions(1).len(),
        1,
        "the child load cursor stays"
    );
    assert_eq!(state.active_child_position(0), Some(0));

    let paged = page(&mut state, 2);
    assert_eq!(paged, (Some(2), vec![0, 1, 2]));
    assert_eq!(focused_position(&state), Some(2));

    let inverted = state.invert_visible(0).expect("invert parent");
    assert_eq!(inverted, (2, Vec::new()));
    assert_eq!(state.selected_positions(1).len(), 1);
    assert_eq!(page(&mut state, 1), (Some(2), Vec::new()));
    let cursor = state.focused_entry();
    assert!(cursor.is_some());

    state.focus_column(1);
    let child_before = state.selected_positions(1);
    let child_all = state.select_visible(1).expect("child entries");
    assert_eq!(child_all, (0, vec![0, 1]));
    assert!(state.selected_positions(0).is_empty());
    assert_ne!(state.selected_positions(1), child_before);
    let parent_marker = state.active_child_position(0);
    assert_eq!(parent_marker, Some(0));
    assert!(state.selected_positions(0).is_empty());
}

#[test]
fn empty_pane_space_does_not_select_the_open_path() {
    let mut state = listing_without_a_load_cursor();
    assert!(state.select(0, 0));
    state.descend(0, location("/fixture/alpha"), RequestId(2));
    state.apply_batch(RequestId(2), Vec::new());
    state.focus_column(1);

    assert_eq!(toggle(&mut state, 1), (CursorToggle::Empty, Vec::new()));
    assert_eq!(
        (state.selected_positions(0), state.active_child_position(0)),
        (vec![0], Some(0))
    );
    assert!(state.focused_entry().is_none());
}

#[test]
fn invert_skips_hidden_entries_and_treats_a_load_cursor_as_empty() {
    let mut state = loaded_with_a_load_cursor(
        "/fixture",
        vec![
            named_entry("/fixture/alpha", "alpha"),
            hidden_entry("/fixture/.secret", ".secret"),
            named_entry("/fixture/bravo", "bravo"),
        ],
    );
    assert!(state.selection_is_load_cursor());
    let (focused, positions) = state.invert_visible(0).expect("invert");
    assert_eq!(state.columns[0].entries[focused].display_name, "alpha");
    assert_eq!(sorted_names(&state, &positions), ["alpha", "bravo"]);
    assert!(!state.selection_is_load_cursor());

    state.set_show_hidden(true);
    let (_, positions) = state.invert_visible(0).expect("invert including hidden");
    assert_eq!(sorted_names(&state, &positions), [".secret"]);
}
