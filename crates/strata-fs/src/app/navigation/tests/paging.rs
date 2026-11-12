// SPDX-License-Identifier: MIT

use super::*;

/// The move each cursor step reports.
type Moves = [(i32, Option<(usize, usize)>)];

fn twelve_items() -> NavigationState {
    loaded(
        "/home",
        (0..12)
            .map(|index| entry(&format!("/home/item-{index:02}")))
            .collect(),
    )
}

fn assert_moves(state: &mut NavigationState, moves: &Moves) {
    assert_steps(state, NavigationState::move_selection, moves);
}

fn assert_pages(state: &mut NavigationState, page: usize, moves: &Moves) {
    assert_steps(
        state,
        |state, direction| state.page_along(direction, page, None),
        moves,
    );
}

/// The focused row and the range a Shift page selects.
fn page_range(state: &mut NavigationState, direction: i32) -> Option<(usize, Vec<usize>)> {
    state
        .extend_page_selection(direction, 5, None)
        .map(|(_, focused, positions)| (focused, positions))
}

#[test]
fn keyboard_selection_skips_hidden_entries_when_hidden_files_are_not_shown() {
    let mut state = loaded("/home", home_with_a_hidden_bravo());

    assert_moves(
        &mut state,
        &[
            (1, Some((0, 0))),
            (1, Some((0, 2))),
            (1, Some((0, 2))),
            (-1, Some((0, 0))),
            (-1, Some((0, 0))),
        ],
    );

    state.set_show_hidden(true);
    assert_moves(&mut state, &[(1, Some((0, 1))), (1, Some((0, 2)))]);
}

#[test]
fn keyboard_selection_is_bounded_and_tracks_the_active_column() {
    let mut state = loaded("/home", vec![entry("/home/one"), entry("/home/two")]);

    assert_moves(
        &mut state,
        &[
            (1, Some((0, 0))),
            (1, Some((0, 1))),
            (1, Some((0, 1))),
            (-1, Some((0, 0))),
            (-1, Some((0, 0))),
        ],
    );
}

#[test]
fn paging_moves_by_a_page_and_stops_at_the_ends() {
    let mut state = twelve_items();

    assert_pages(
        &mut state,
        5,
        &[
            (1, Some((0, 0))),
            (1, Some((0, 5))),
            (1, Some((0, 10))),
            (1, Some((0, 11))),
            (-1, Some((0, 6))),
            (-1, Some((0, 1))),
            (-1, Some((0, 0))),
        ],
    );
    assert_eq!(state.selected_entries().len(), 1);
}

#[test]
fn shift_paging_extends_from_the_anchor_and_contracts_back() {
    let mut state = twelve_items();
    assert!(state.select(0, 4));

    assert_steps(
        &mut state,
        page_range,
        &[
            (1, Some((9, (4..=9).collect()))),
            (1, Some((11, (4..=11).collect()))),
            (-1, Some((6, (4..=6).collect()))),
        ],
    );
    assert_eq!(
        page_range(&mut state, -1),
        Some((1, (1..=4).collect())),
        "crossing the anchor flips the range"
    );
    assert_eq!(state.selected_entries().len(), 4);

    let cleared = state.clear_active_selection();
    assert!(cleared.is_some());
    assert_eq!(
        page_range(&mut state, 1),
        Some((6, (1..=6).collect())),
        "a cleared fill restarts the range at the cursor"
    );
}

#[test]
fn shift_paging_follows_display_order_and_skips_hidden_entries() {
    let mut state = loaded(
        "/home",
        vec![
            named_entry("/home/a.txt", "a.txt"),
            hidden_entry("/home/b.txt", "b.txt"),
            named_entry("/home/c.json", "c.json"),
            named_entry("/home/d.txt", "d.txt"),
        ],
    );
    assert!(state.select(0, 0));
    assert_eq!(
        state.extend_page_selection(1, 1, None),
        Some((0, 2, vec![0, 2]))
    );
    assert!(state.select(0, 0));
    assert_eq!(
        state.extend_page_selection(1, 1, Some(&[0, 3, 2])),
        Some((0, 3, vec![0, 3]))
    );
}

#[test]
fn paging_skips_hidden_entries_when_hidden_files_are_not_shown() {
    let mut entries = home_with_a_hidden_bravo();
    entries.push(named_entry("/home/delta", "delta"));
    let mut state = loaded("/home", entries);

    assert!(state.select(0, 0));
    assert_pages(&mut state, 1, &[(1, Some((0, 2))), (-1, Some((0, 0)))]);
}

#[test]
fn paging_by_usize_max_jumps_to_the_first_or_last_visible_entry() {
    let listing = || {
        loaded(
            "/home",
            vec![
                hidden_entry("/home/alpha", "alpha"),
                named_entry("/home/bravo", "bravo"),
                named_entry("/home/charlie", "charlie"),
                hidden_entry("/home/delta", "delta"),
            ],
        )
    };

    let mut state = listing();
    assert!(state.select(0, 2));
    assert_pages(
        &mut state,
        usize::MAX,
        &[(1, Some((0, 2))), (-1, Some((0, 1)))],
    );

    for (direction, target) in [(-1, 1), (1, 2)] {
        let mut state = listing();
        assert_eq!(state.columns[0].selected, None);
        assert_eq!(
            state.page_along(direction, usize::MAX, None),
            Some((0, target)),
            "jump {direction} without a cursor"
        );
    }
}

#[test]
fn paging_an_empty_column_keeps_the_selection_unchanged() {
    let mut state = loaded("/home", Vec::new());

    assert_eq!(state.page_along(1, 4, None), None);
}

#[test]
fn paging_along_visual_order_follows_display_order_not_source_indices() {
    let mut state = loaded(
        "/home",
        children("/home", &["a.txt", "b.json", "c.txt", "d.json"]),
    );
    assert!(state.select(0, 0));
    let visual_order = [0, 2, 1, 3];
    assert_eq!(
        state.page_along(1, 2, Some(&visual_order)),
        Some((0, 1)),
        "two steps from a.txt along txt-then-json lands on b.json"
    );
    assert_eq!(state.page_along(1, 1, Some(&visual_order)), Some((0, 3)));
    assert_eq!(state.page_along(-1, 2, Some(&visual_order)), Some((0, 2)));
}
