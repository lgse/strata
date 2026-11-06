// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Arrival {
    Batch,
    Snapshot,
}

#[derive(Clone, Copy, Debug)]
enum ListingChange {
    Batch,
    Upsert,
    Remove,
    Move,
    NewEntries,
    Removals,
    Sort,
}

#[derive(Clone, Copy, Debug)]
enum FillChange {
    Widen,
    Narrow,
    HideHidden,
    Reload,
    SelectAll,
}

/// Asserts that the cached fill, a full rescan of the listing, and the selected names agree.
fn assert_tracked_fill(
    state: &NavigationState,
    positions: &[usize],
    names: &[&str],
    context: &str,
) {
    assert_eq!(state.selected_positions(0), positions, "{context}");
    assert_eq!(scanned_positions(state, 0), positions, "{context}");
    assert_eq!(selected_names(state), names, "{context}");
}

fn alpha_and_bravo() -> Vec<FileEntry> {
    children("/fixture", &["alpha", "bravo"])
}

/// Lists alpha's location a second time, under the name charlie.
fn repeat_alpha(state: &mut NavigationState, arrival: Arrival) {
    let repeat = named_entry("/fixture/alpha", "charlie");
    match arrival {
        Arrival::Batch => {
            state.apply_batch(RequestId(1), vec![repeat]);
        }
        Arrival::Snapshot => {
            let mut entries = alpha_and_bravo();
            entries.push(repeat);
            state.install_snapshot(RequestId(1), entries);
        }
    }
}

fn assert_reports_select_every_repeated_row(state: &mut NavigationState, arrival: Arrival) {
    let names = ["alpha", "bravo", "charlie"];
    for (commit, reported, positions) in [
        (true, &[0, 1][..], &[0, 1, 2][..]),
        (false, &[0, 1], &[0, 1, 2]),
        (true, &[1], &[1]),
        (true, &[1, 2], &[0, 1, 2]),
    ] {
        if commit {
            state.commit_selection();
        }
        assert!(state.set_selection(0, reported, reported.last().copied()));
        let expected: Vec<_> = positions.iter().map(|position| names[*position]).collect();
        let context = format!("{arrival:?} {reported:?}");
        assert_eq!(state.selected_positions(0), positions, "{context}");
        assert_eq!(selected_names(state), expected, "{context}");
        assert_eq!(command_names(state, 0), expected, "{context}");
    }
}

fn change_the_listing(state: &mut NavigationState, watched: &Location, change: ListingChange) {
    let arrival = || named_entry("/home/aardvark", "aardvark");
    match change {
        ListingChange::Batch => {
            state.apply_batch(RequestId(1), vec![arrival()]);
        }
        ListingChange::Upsert => {
            state.apply_directory_change(0, watched, DirectoryChange::Upsert(arrival()));
        }
        ListingChange::Remove => {
            state.apply_directory_change(
                0,
                watched,
                DirectoryChange::Remove(location("/home/alpha")),
            );
        }
        ListingChange::Move => {
            let change = DirectoryChange::Move {
                from: location("/home/bravo"),
                entry: named_entry("/home/echo", "echo"),
            };
            state.apply_directory_change(0, watched, change);
        }
        ListingChange::NewEntries => {
            state
                .apply_new_entries_batch(0, watched, vec![arrival()])
                .expect("new entries apply");
        }
        ListingChange::Removals => {
            state.apply_removals_batch(
                0,
                watched,
                [location("/home/alpha"), location("/home/charlie")],
            );
        }
        ListingChange::Sort => {
            state.apply_sort_preferences(0, descending());
        }
    }
}

/// `/home` with a hidden `.env` followed by alpha, bravo, charlie and delta.
fn home_with_a_hidden_env() -> Vec<FileEntry> {
    let mut entries = vec![hidden_entry("/home/.env", ".env")];
    entries.extend(children("/home", &["alpha", "bravo", "charlie", "delta"]));
    entries
}

fn change_the_fill(state: &mut NavigationState, watched: &Location, change: FillChange) {
    match change {
        FillChange::Widen => assert!(state.set_selection(0, &[0, 1, 2, 4], Some(1))),
        FillChange::Narrow => assert!(state.set_selection(0, &[2], Some(2))),
        FillChange::HideHidden => state.set_show_hidden(false),
        FillChange::Reload => {
            state.refresh_column(0, RequestId(2));
            let mut entries = home_with_a_hidden_env();
            entries.insert(1, named_entry("/home/aardvark", "aardvark"));
            state.install_snapshot(RequestId(2), entries);
        }
        FillChange::SelectAll => {
            state.select_all(0);
            state.apply_directory_change(
                0,
                watched,
                DirectoryChange::Remove(location("/home/charlie")),
            );
        }
    }
}

#[test]
fn a_repeated_location_is_selected_on_every_row_that_lists_it() {
    for arrival in [Arrival::Batch, Arrival::Snapshot] {
        let mut state = loaded("/fixture", alpha_and_bravo());
        state.commit_selection();
        assert!(state.set_selection(0, &[0, 1], Some(1)));
        repeat_alpha(&mut state, arrival);

        assert_reports_select_every_repeated_row(&mut state, arrival);

        assert!(state.select(0, 0));
        assert_eq!(state.selected_positions(0), [0, 2], "{arrival:?}");
        state.commit_selection();
        assert!(state.set_selection(0, &[0, 1], Some(1)));
        assert_eq!(state.selected_positions(0), [0, 1, 2], "{arrival:?}");
    }
}

#[test]
fn a_lone_selected_cursor_follows_each_batch_onto_every_row_of_its_location() {
    let mut state = loaded("/fixture", vec![child("/fixture", "bravo")]);
    assert!(state.select(0, 0));
    assert_eq!(state.selected_positions(0), [0]);

    for (arrival, positions, names) in [
        (child("/fixture", "alpha"), &[1][..], &["bravo"][..]),
        (
            named_entry("/fixture/bravo", "charlie"),
            &[1, 2],
            &["bravo", "charlie"],
        ),
    ] {
        state.apply_batch(RequestId(1), vec![arrival]);
        assert_tracked_fill(&state, positions, names, "");
    }
}

#[test]
fn a_fill_is_matched_by_location_when_the_listing_repeats_one_and_drops_another() {
    for (commit, reported, positions, count) in [
        (true, &[0, 1, 3][..], &[0, 1, 2, 3][..], 3),
        (false, &[0, 2, 3], &[0, 2, 3], 2),
    ] {
        let mut state = loaded(
            "/fixture",
            children("/fixture", &["alpha", "bravo", "delta", "echo"]),
        );
        state.commit_selection();
        assert!(state.set_selection(0, &[0, 2, 3], Some(3)));
        let mut relisted = alpha_and_bravo();
        relisted.push(named_entry("/fixture/alpha", "charlie"));
        relisted.push(child("/fixture", "delta"));
        state.install_snapshot(RequestId(1), relisted);
        assert_eq!(state.selected_positions(0), [0, 2, 3]);

        if commit {
            state.commit_selection();
        }
        assert!(state.set_selection(0, reported, Some(3)));
        assert_eq!(state.selected_positions(0), positions, "{reported:?}");
        assert_eq!(state.selected_count(), count, "{reported:?}");
    }
}

#[test]
fn a_fill_follows_every_listing_change() {
    let watched = location("/home");
    for (change, positions, names) in [
        (ListingChange::Batch, &[2, 4][..], &["bravo", "delta"][..]),
        (ListingChange::Upsert, &[2, 4], &["bravo", "delta"]),
        (ListingChange::Remove, &[0, 2], &["bravo", "delta"]),
        (ListingChange::Move, &[2, 3], &["delta", "echo"]),
        (ListingChange::NewEntries, &[2, 4], &["bravo", "delta"]),
        (ListingChange::Removals, &[0, 1], &["bravo", "delta"]),
        (ListingChange::Sort, &[0, 2], &["delta", "bravo"]),
    ] {
        let mut state = loaded(
            "/home",
            children("/home", &["alpha", "bravo", "charlie", "delta"]),
        );
        assert!(state.set_selection(0, &[1, 3], Some(3)));
        assert_eq!(state.selected_positions(0), [1, 3]);

        change_the_listing(&mut state, &watched, change);

        let context = format!("{change:?}");
        assert_tracked_fill(&state, positions, names, &context);
        assert_eq!(command_names(&state, 0), names, "{context}");
    }
}

#[test]
fn cached_selection_positions_follow_every_selection_and_visibility_change() {
    let watched = location("/home");
    for (change, positions, names) in [
        (
            FillChange::Widen,
            &[0, 1, 2, 4][..],
            &[".env", "alpha", "bravo", "delta"][..],
        ),
        (FillChange::Narrow, &[2], &["bravo"]),
        (FillChange::HideHidden, &[2, 4], &["bravo", "delta"]),
        (FillChange::Reload, &[0, 3, 5], &[".env", "bravo", "delta"]),
        (
            FillChange::SelectAll,
            &[0, 1, 2, 3],
            &[".env", "alpha", "bravo", "delta"],
        ),
    ] {
        let mut state = NavigationState::default();
        state.set_show_hidden(true);
        list(&mut state, "/home", home_with_a_hidden_env());
        assert!(state.set_selection(0, &[0, 2, 4], Some(4)));
        assert_eq!(state.selected_positions(0), [0, 2, 4], "{change:?}: primed");

        change_the_fill(&mut state, &watched, change);

        assert_tracked_fill(&state, positions, names, &format!("{change:?}"));
    }
}
