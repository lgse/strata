// SPDX-License-Identifier: MIT

use super::*;

/// One step of a walk: the row the cursor lands on, the fill expected there, and why.
type Walk = (usize, &'static [&'static str], &'static str);

/// Grouped display order: delta, echo, then alpha, bravo, charlie.
const GROUPED: [usize; 5] = [3, 4, 0, 1, 2];

const ALL_BUT_BRAVO: &[&str] = &["alpha", "charlie", "delta", "echo"];

const GROW_AND_CONTRACT: &[Walk] = &[
    (0, &["alpha", "delta", "echo"], "the range grows"),
    (1, &["alpha", "bravo", "delta", "echo"], "the range grows"),
    (
        4,
        &["delta", "echo"],
        "walking back contracts to the base fill",
    ),
];

const UNSET_WALK: &[Walk] = &[
    (2, &["delta", "echo"], "unset clears the walked span"),
    (
        4,
        &["charlie", "delta"],
        "unset walks backward across the displayed groups",
    ),
];

fn five_entry_listing(state: &mut NavigationState) {
    list(
        state,
        "/fixture",
        children("/fixture", &["alpha", "bravo", "charlie", "delta", "echo"]),
    );
}

fn fill(state: &NavigationState, depth: usize) -> Vec<String> {
    let mut names: Vec<_> = state
        .selected_positions(depth)
        .into_iter()
        .map(|position| state.columns[depth].entries[position].display_name.clone())
        .collect();
    names.sort();
    names
}

fn parent_and_child_fills(state: &NavigationState) -> (Vec<String>, Vec<String>) {
    (fill(state, 0), fill(state, 1))
}

fn walk_to(state: &mut NavigationState, position: usize) {
    state.place_cursor(0, position).expect("listed position");
    state.refresh_visual(Some(&GROUPED));
}

fn assert_walk(state: &mut NavigationState, walk: &[Walk]) {
    for (position, expected, reason) in walk {
        walk_to(state, *position);
        assert_eq!(fill(state, 0), *expected, "at {position}: {reason}");
    }
}

#[test]
fn visual_ranges_cross_type_groups_and_contract_without_selecting_filtered_entries() {
    let mut state = loaded(
        "/fixture",
        children("/fixture", &["a.txt", "b.txt", "c.txt", "d.json", "e.json"]),
    );
    state.select(0, 4);
    let visual_order = [3, 4, 0, 2];
    assert_steps(
        &mut state,
        |state, focused| state.extend_visual_selection(0, focused, &visual_order),
        &[
            (2, Some(vec![4, 0, 2])),
            (0, Some(vec![4, 0])),
            (4, Some(vec![4])),
            (3, Some(vec![3, 4])),
            (1, None),
        ],
    );
    assert_eq!(state.extend_visual_selection(0, 99, &[4, 99]), None);
    assert_eq!(state.selected_entries().len(), 2);
}

#[test]
fn a_filtered_out_range_anchor_restarts_at_the_visible_target() {
    let mut state = loaded("/fixture", children("/fixture", &["a", "b"]));
    state.select(0, 0);
    assert_eq!(state.extend_visual_selection(0, 1, &[1]), Some(vec![1]));
    assert_eq!(
        state.extend_visual_selection(0, 0, &[0, 1]),
        Some(vec![0, 1])
    );
}

#[test]
fn the_range_anchor_is_readable_and_replaceable_by_position() {
    let mut state = loaded_with_a_load_cursor("/fixture", children("/fixture", &["a", "b", "c"]));
    assert_eq!(state.selection_anchor_position(0), Some(0));
    for (depth, position, accepted) in [(0, 2, true), (0, 9, false), (1, 0, false)] {
        assert_eq!(
            state.set_selection_anchor(depth, position),
            accepted,
            "anchor {depth}:{position}"
        );
        assert_eq!(state.selection_anchor_position(0), Some(2));
    }
    assert_eq!(
        state.extend_visual_selection(0, 1, &[0, 1, 2]),
        Some(vec![1, 2])
    );
}

#[test]
fn visual_ranges_add_and_subtract_the_walked_span_in_displayed_order() {
    let mut state = NavigationState::default();
    five_entry_listing(&mut state);
    assert!(state.install_pane_fill(0, &[4], 3));
    assert_eq!(fill(&state, 0), ["echo"]);

    let (_, cursor, _) = state
        .start_visual(VisualKind::Select, Some(&GROUPED))
        .expect("range at the cursor");
    assert_eq!(
        (cursor, state.visual_kind(), fill(&state, 0)),
        (3, Some(VisualKind::Select), owned(&["delta", "echo"]))
    );
    assert_walk(&mut state, GROW_AND_CONTRACT);
    walk_to(&mut state, 1);

    let (_, cursor, _) = state
        .toggle_visual_cursor(Some(&GROUPED))
        .expect("toggle in the range");
    assert_eq!(cursor, 1, "Space does not advance in visual mode");
    assert_eq!(fill(&state, 0), ["alpha", "delta", "echo"]);
    let toggled_and_grown = (
        2,
        ALL_BUT_BRAVO,
        "a toggled item stays flipped as the range grows",
    );
    assert_walk(&mut state, &[toggled_and_grown]);

    assert!(state.leave_visual());
    assert_eq!(state.visual_kind(), None);
    let after_leaving = (
        3,
        ALL_BUT_BRAVO,
        "ordinary motion after leaving keeps the fill",
    );
    assert_walk(&mut state, &[after_leaving]);

    walk_to(&mut state, 0);
    state
        .start_visual(VisualKind::Unset, Some(&GROUPED))
        .expect("unset range at the new cursor");
    assert_eq!(fill(&state, 0), ["charlie", "delta", "echo"]);
    assert_walk(&mut state, UNSET_WALK);
    assert!(state.leave_visual());

    state
        .start_visual(VisualKind::Select, Some(&GROUPED))
        .expect("new range at the cursor");
    let new_anchor = (
        0,
        ALL_BUT_BRAVO,
        "another v anchors at the new cursor, not the old range",
    );
    assert_walk(&mut state, &[new_anchor]);
}

#[test]
fn visual_ranges_start_on_a_load_cursor_and_leave_other_columns_alone() {
    let mut state = listing_with_the_first_entry_selected();
    state.descend(0, location("/fixture/alpha"), RequestId(2));
    state.apply_batch(
        RequestId(2),
        children("/fixture/alpha", &["one", "two", "three"]),
    );
    assert!(state.select(1, 0));
    assert!(state.install_pane_fill(1, &[0, 2], 1));
    state.focus_column(0);
    assert!(state.selection_is_load_cursor());

    state
        .start_visual(VisualKind::Select, None)
        .expect("range on the load cursor");
    assert!(!state.selection_is_load_cursor());
    assert_eq!(fill(&state, 0), ["alpha"]);
    state.page_cursor(1, 1, None);
    state.refresh_visual(None);
    assert_eq!(
        parent_and_child_fills(&state),
        (owned(&["alpha", "bravo"]), owned(&["one", "two"])),
        "the child fill is untouched"
    );
    assert_eq!(state.active_child_position(0), Some(0));

    state.focus_column(1);
    assert_eq!(
        state.visual_kind(),
        None,
        "another pane does not continue the range"
    );
    state.focus_column(0);
    assert_eq!(
        state.visual_kind(),
        None,
        "returning to the pane does not resume the range"
    );
    assert_eq!(state.refresh_visual(None), None);
    assert_eq!(
        parent_and_child_fills(&state),
        (owned(&["alpha", "bravo"]), owned(&["one", "two"]))
    );
}

#[test]
fn visual_ranges_end_without_an_anchor_listing_or_pointer_commit() {
    let mut state = NavigationState::default();
    state.navigate(location("/empty"), RequestId(1));
    state.apply_batch(RequestId(1), Vec::new());
    assert_eq!(state.start_visual(VisualKind::Select, None), None);
    assert_eq!(state.visual_kind(), None);

    five_entry_listing(&mut state);
    assert!(state.install_pane_fill(0, &[], 1));
    state
        .start_visual(VisualKind::Select, None)
        .expect("range at bravo");
    state.reload_column(0, RequestId(3));
    state.apply_batch(
        RequestId(3),
        children("/fixture", &["alpha", "charlie", "delta"]),
    );
    assert_eq!(
        state.visual_kind(),
        None,
        "a rebuilt listing without the anchor"
    );
    state.place_cursor(0, 2);
    assert_eq!(state.refresh_visual(None), None);

    state
        .start_visual(VisualKind::Unset, None)
        .expect("range at delta");
    state.commit_selection();
    assert!(state.set_selection(0, &[0], Some(0)));
    assert_eq!(
        state.visual_kind(),
        None,
        "a pointer selection ends the range"
    );
    state.place_cursor(0, 2);
    assert_eq!(state.refresh_visual(None), None);
    assert_eq!(fill(&state, 0), ["alpha"]);

    state
        .start_visual(VisualKind::Select, None)
        .expect("range at delta");
    state.navigate(location("/elsewhere"), RequestId(4));
    assert_eq!(state.visual_kind(), None);
    assert_eq!(state.refresh_visual(None), None);
}
