// SPDX-License-Identifier: MIT

use super::*;

/// Delivers the snapshot for a reload of `/fixture` that now lists `names`.
fn relist(state: &mut NavigationState, request: u64, names: &[&str]) -> Option<usize> {
    state.install_snapshot(RequestId(request), children("/fixture", names))
}

fn selected_locations(state: &NavigationState) -> Vec<Location> {
    state
        .selected_entries()
        .into_iter()
        .map(|entry| entry.location)
        .collect()
}

#[test]
fn a_rename_rebases_the_pending_selection_during_a_refresh() {
    let mut state = loaded("/home", vec![named_entry("/home/old", "old")]);
    state.select(0, 0);
    state.reload_column(0, RequestId(2));
    state.apply_directory_change(
        0,
        &location("/home"),
        DirectoryChange::Move {
            from: location("/home/old"),
            entry: named_entry("/home/new", "new"),
        },
    );
    state.install_snapshot(RequestId(2), vec![named_entry("/home/new", "new")]);
    assert_eq!(state.selected_positions(0), [0]);
    assert_eq!(
        state.focused_entry().expect("focused entry").2.location,
        location("/home/new")
    );
}

#[test]
fn a_rename_rebases_every_pending_reveal_target() {
    let mut state = NavigationState::default();
    state.navigate(location("/home/old"), RequestId(1));
    state.select_locations_on_load(0, vec![location("/home/old/b"), location("/home/old/a")]);

    state.relocate_column(0, location("/home/new"), RequestId(2));
    state.install_snapshot(RequestId(2), children("/home/new", &["a", "b"]));

    assert_eq!(state.selected_positions(0), [0, 1]);
    assert_eq!(
        state.focused_entry().expect("focused entry").2.location,
        location("/home/new/b")
    );
}

#[test]
fn truncated_load_state_survives_until_reload() {
    let mut state = NavigationState::default();
    state.navigate(location("/partial"), RequestId(1));

    assert_eq!(
        state.finish(RequestId(1), true, None, None),
        Some((0, false))
    );
    assert!(state.columns[0].truncated);

    state.reload_column(0, RequestId(2));
    assert!(!state.columns[0].truncated);
}

#[test]
fn reload_clears_the_resolved_delete_capability() {
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));

    assert_eq!(
        state.finish(RequestId(1), false, None, Some(false)),
        Some((0, false))
    );
    assert_eq!(state.can_delete_at(0), Some(false));

    state.reload_column(0, RequestId(2));
    assert_eq!(state.can_delete_at(0), None);
}

#[test]
fn reload_restores_a_multi_selection_after_snapshot() {
    let mut state = listing_without_a_load_cursor();
    assert!(state.set_selection(0, &[0, 2], Some(2)));

    state.reload_column(0, RequestId(2));
    assert!(state.selected_positions(0).is_empty());
    assert_eq!(relist(&mut state, 2, &ABC), Some(0));
    assert_eq!(fill_and_focus(&state, 0), (vec![0, 2], Some((0, Some(2)))));
}

#[test]
fn reload_does_not_select_an_unselected_focus() {
    for positions in [vec![], vec![0], vec![0, 1]] {
        let mut state = listing_without_a_load_cursor();
        assert!(state.set_selection(0, &positions, Some(2)));
        let expected = positions
            .iter()
            .map(|&position| state.columns[0].entries[position].location.clone())
            .collect::<Vec<_>>();
        assert_eq!(state.selected_positions(0), positions);
        assert_eq!(selected_locations(&state), expected);
        state.reload_column(0, RequestId(2));
        relist(&mut state, 2, &ABC);
        assert_eq!(
            fill_and_focus(&state, 0),
            (positions.clone(), Some((0, Some(2))))
        );
        assert_eq!(selected_locations(&state), expected);
    }
}

#[test]
fn reload_drops_selection_members_that_left_the_listing() {
    let mut state = listing_without_a_load_cursor();
    assert!(state.set_selection(0, &[0, 2], Some(2)));

    state.reload_column(0, RequestId(2));
    assert_eq!(relist(&mut state, 2, &["alpha", "bravo"]), Some(0));
    assert_eq!(fill_and_focus(&state, 0), (vec![0], Some((0, Some(0)))));
}

#[test]
fn reload_moves_a_removed_cursor_to_its_neighbour() {
    let mut failures = Vec::new();
    for (cursor, remaining, expected, reloads) in [
        (2, &["alpha", "bravo"][..], Some(1), 1),
        (1, &["alpha", "charlie"][..], Some(1), 1),
        (0, &[][..], None, 1),
        // A burst restarts the reload before the first one lists anything.
        (1, &["alpha", "charlie"][..], Some(1), 2),
    ] {
        let mut state = listing_without_a_load_cursor();
        assert!(state.set_selection(0, &[cursor], Some(cursor)));

        for reload in 0..reloads {
            state.reload_column(0, RequestId(2 + reload));
        }
        relist(&mut state, 1 + reloads, remaining);

        let landed = fill_and_focus(&state, 0);
        if landed != (Vec::new(), Some((0, expected))) {
            failures.push(format!(
                "cursor on entry {cursor}, {remaining:?} remain after {reloads} reloads: selection \
                 and focus {landed:?}, expected cursor {expected:?} with nothing selected",
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn reload_leaves_no_neighbour_cursor_beside_an_open_child_column() {
    let mut state = listing_without_a_load_cursor();
    assert!(state.set_selection(0, &[1], Some(1)));
    assert!(state.descend(0, location("/fixture/bravo"), RequestId(2)));

    state.reload_column(0, RequestId(3));
    relist(&mut state, 3, &["alpha", "charlie"]);

    assert_eq!(state.columns[0].selected, None);
}
