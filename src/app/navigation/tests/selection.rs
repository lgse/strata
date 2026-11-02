// SPDX-License-Identifier: MIT

use super::*;

/// A GTK selection report: the selected rows and the focused row.
type Report<'a> = (&'a [usize], Option<usize>, &'a str);

/// Reports a selection for the root column and returns the fill it leaves.
fn report_selection(
    state: &mut NavigationState,
    positions: &[usize],
    focused: Option<usize>,
) -> Vec<usize> {
    assert!(state.set_selection(0, positions, focused));
    state.selected_positions(0)
}

/// Fills alpha and bravo, then moves the cursor alone onto charlie.
fn fill_beside_the_cursor(state: &mut NavigationState) {
    assert!(state.set_selection(0, &[0, 1], Some(0)));
    assert!(state.place_cursor(0, 2).is_some());
}

fn assert_load_cursor_after(state: &mut NavigationState, reports: &[Report], load_cursor: bool) {
    for (positions, focused, message) in reports {
        assert!(state.set_selection(0, positions, *focused), "{message}");
        assert_eq!(state.selection_is_load_cursor(), load_cursor, "{message}");
    }
}

#[test]
fn empty_selection_sync_preserves_the_keyboard_cursor() {
    let mut state = listing_without_a_load_cursor();
    state.select(0, 1);
    assert_eq!(state.clear_active_selection(), Some((0, 1)));
    for (focused, direction, moved) in [(None, 1, (0, 2)), (Some(1), -1, (0, 0))] {
        assert!(state.set_selection(0, &[], focused));
        assert_eq!(
            fill_and_focus(&state, 0),
            (vec![], Some((0, Some(1)))),
            "{focused:?}"
        );
        assert_eq!(state.move_selection(direction), Some(moved), "{focused:?}");
    }
}

#[test]
fn multi_selection_tracks_entries_and_replaces_cleanly() {
    let mut state = listing_without_a_load_cursor();

    assert!(state.set_selection(0, &[0, 2], Some(2)));
    assert_eq!(
        (selected_names(&state), focused_position(&state)),
        (owned(&["alpha", "charlie"]), Some(2))
    );

    assert!(state.select(0, 1));
    assert_eq!(selected_names(&state), ["bravo"]);
}

#[test]
fn successive_fills_select_exactly_the_reported_entries() {
    let listed = ["alpha", "bravo", "charlie", "delta", "echo"];
    let mut state = loaded("/fixture", children("/fixture", &listed));

    for (positions, names) in [
        (&[0, 1, 2, 3][..], &listed[..4]),
        (&[1, 2, 3, 4], &listed[1..]),
        (&[0, 4], &["alpha", "echo"]),
        (&[0, 1, 4], &["alpha", "bravo", "echo"]),
        (&[], &[]),
    ] {
        state.commit_selection();
        assert!(state.set_selection(0, positions, positions.last().copied()));
        assert_eq!(
            (state.selected_positions(0), state.selected_count()),
            (positions.to_vec(), positions.len())
        );
        assert_eq!(selected_names(&state), names);
    }
}

#[test]
fn filled_entries_distinguishes_a_committed_fill_from_load_cursor_and_selectionless_cursor() {
    let command_count_and_fill = |state: &NavigationState| {
        (
            state.command_entries(0).len(),
            names(&state.filled_entries(0)),
        )
    };
    let mut state = listing_with_the_first_entry_selected();
    assert_eq!(
        command_count_and_fill(&state),
        (1, Vec::new()),
        "a load cursor is not a fill"
    );

    assert!(state.select(0, 1));
    assert_eq!(names(&state.filled_entries(0)), ["bravo"]);

    let cleared = state.clear_active_selection();
    assert!(cleared.is_some());
    assert_eq!(
        command_count_and_fill(&state),
        (1, Vec::new()),
        "command_entries falls back to the cursor; a cleared selection leaves no filled entries"
    );
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
    assert_eq!(focused_position(&state), Some(1));
    assert!(
        state.selection_is_load_cursor(),
        "the first visible item after load is a cursor, not a user selection"
    );

    assert_load_cursor_after(
        &mut state,
        &[
            (
                &[1],
                Some(1),
                "echoing the load selection must not treat it as a user pick",
            ),
            (
                &[],
                None,
                "an empty GTK echo is not itself a paste-into target",
            ),
            (
                &[1],
                Some(1),
                "restoring the load cursor after an empty echo is still a load cursor",
            ),
            (
                &[0],
                Some(0),
                "GTK focusing another row without a user pick must not arm paste-into",
            ),
        ],
        true,
    );

    state.commit_selection();
    assert_load_cursor_after(
        &mut state,
        &[
            (&[0], Some(0), "choosing another item is a user selection"),
            (&[1], Some(1), "a later pick stays a user selection"),
        ],
        false,
    );
}

#[test]
fn a_cleared_cursor_report_selects_and_a_filled_one_does_not() {
    let mut state = listing_without_a_load_cursor();
    assert!(state.set_selection(0, &[0, 2], Some(2)));
    assert_eq!(state.clear_active_selection(), Some((0, 2)));
    assert!(state.selected_positions(0).is_empty());

    for message in [
        "clicking the still-focused file after a clear selects it",
        "repeating the current locations does not rewrite the fill",
    ] {
        assert_eq!(
            report_selection(&mut state, &[2], Some(2)),
            [2],
            "{message}"
        );
    }

    fill_beside_the_cursor(&mut state);
    assert_eq!(state.selected_positions(0), [0, 1]);
    assert_eq!(
        report_selection(&mut state, &[2], Some(2)),
        [0, 1],
        "an uncommitted cursor-only report must not replace a different fill"
    );
    state.commit_selection();
    assert_eq!(
        report_selection(&mut state, &[2], Some(2)),
        [2],
        "an explicit commit of the focused file selects it"
    );

    fill_beside_the_cursor(&mut state);
    assert_eq!(
        report_selection(&mut state, &[0, 1], Some(2)),
        [0, 1],
        "repeating an unchanged fill while the cursor is outside it is an echo"
    );
    assert_eq!(focused_position(&state), Some(2));
}

#[test]
fn selected_count_reports_without_cloning_entries() {
    let mut state = listing_without_a_load_cursor();
    assert_eq!(state.selected_count(), 0);
    assert!(state.set_selection(0, &[0, 2], Some(2)));
    assert_eq!(state.selected_count(), 2);
}
