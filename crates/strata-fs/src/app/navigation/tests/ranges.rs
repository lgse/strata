// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn keyboard_range_selection_extends_and_contracts_from_its_anchor() {
    let mut state = listing_without_a_load_cursor();
    assert!(state.select(0, 0));

    assert_steps(
        &mut state,
        extend,
        &[
            (1, Some(vec![0, 1])),
            (1, Some(vec![0, 1, 2])),
            (-1, Some(vec![0, 1])),
        ],
    );
    assert_eq!(state.selected_entries().len(), 2);
}

#[test]
fn extending_from_no_selection_starts_at_a_single_edge_entry() {
    for (direction, edge, widened) in [(1, vec![0], vec![0, 1]), (-1, vec![2], vec![1, 2])] {
        let mut state = listing_without_a_load_cursor();
        assert_steps(
            &mut state,
            extend,
            &[(direction, Some(edge)), (direction, Some(widened))],
        );
    }
}

#[test]
fn extending_from_an_escape_cleared_cursor_starts_on_that_entry() {
    let mut state = listing_with_the_first_entry_selected();
    assert_eq!(fill_and_focus(&state, 0), (vec![0], Some((0, Some(0)))));

    assert_eq!(state.clear_active_selection(), Some((0, 0)));
    assert_eq!(
        fill_focus_and_anchor(&state),
        (vec![], Some((0, Some(0))), Some(0))
    );

    let restarted = extend(&mut state, 1);
    assert_eq!(
        (restarted, state.selection_anchor_position(0)),
        (Some(vec![0]), Some(0))
    );
    assert_eq!(extend(&mut state, 1), Some(vec![0, 1]));
}

#[test]
fn extending_from_an_escape_cleared_range_starts_on_the_cursor() {
    let mut state = listing_with_the_first_entry_selected();
    let range = extend(&mut state, 1);
    assert_eq!(
        (
            range,
            state.active_focus(),
            state.selection_anchor_position(0)
        ),
        (Some(vec![0, 1]), Some((0, Some(1))), Some(0))
    );

    assert_eq!(state.clear_active_selection(), Some((0, 1)));
    assert_eq!(
        fill_focus_and_anchor(&state),
        (vec![], Some((0, Some(1))), Some(0))
    );

    let restarted = extend(&mut state, 1);
    assert_eq!(
        (restarted, state.selection_anchor_position(0)),
        (Some(vec![1]), Some(1))
    );
    assert_eq!(extend(&mut state, 1), Some(vec![1, 2]));
}

#[test]
fn keyboard_selection_extension_skips_hidden_entries() {
    let mut state = loaded("/home", home_with_a_hidden_bravo());
    assert!(state.select(0, 0));

    assert_eq!(extend(&mut state, 1), Some(vec![0, 2]));
    let selected = state.selected_entries();
    assert_eq!(selected.len(), 2);
    assert!(!selected.iter().any(|entry| entry.is_hidden));
}
