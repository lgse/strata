// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Delivery {
    Snapshot,
    OneBatch,
    LaterBatch,
}

/// A preferred location the listing cannot select, and why.
struct Fallback {
    reason: &'static str,
    preferred: &'static str,
    listing: Vec<FileEntry>,
}

fn docs(name: &str) -> FileEntry {
    child("/docs", name)
}

fn preferring(preferred: &str) -> NavigationState {
    let mut state = NavigationState::default();
    state.navigate(location("/docs"), RequestId(1));
    state.prefer_location_on_load(0, location(preferred));
    state
}

/// Delivers a1, b2 and c3, reporting a selection made before b2 arrived.
fn deliver_docs(state: &mut NavigationState, delivery: Delivery) -> Option<String> {
    let all = || vec![docs("a1"), docs("b2"), docs("c3")];
    match delivery {
        Delivery::Snapshot => {
            state.install_snapshot(RequestId(1), all());
            None
        }
        Delivery::OneBatch => {
            state.apply_batch(RequestId(1), all());
            None
        }
        Delivery::LaterBatch => {
            state.apply_batch(RequestId(1), vec![docs("a1")]);
            let early = (fill_and_focus(state, 0) != (Vec::new(), Some((0, None)))).then(|| {
                format!(
                    "{delivery:?}: the first batch selected {:?} before b2 arrived",
                    state.selected_positions(0)
                )
            });
            state.apply_batch(RequestId(1), vec![docs("b2"), docs("c3")]);
            early
        }
    }
}

/// Delivers `listing`, reporting a selection made before a batched listing completed.
fn deliver_listing(
    state: &mut NavigationState,
    listing: &[FileEntry],
    case: &str,
    batched: bool,
) -> Option<String> {
    if !batched {
        state.install_snapshot(RequestId(1), listing.to_vec());
        return None;
    }
    state.apply_batch(RequestId(1), listing.to_vec());
    let early = (!state.selected_positions(0).is_empty()).then(|| {
        format!(
            "{case}: selected {:?} before the listing completed",
            state.selected_positions(0)
        )
    });
    state.finish(RequestId(1), false, None, None);
    early
}

/// Every way a missing or hidden preferred entry fails to fall back to the first visible one.
fn fallback_failures(fallback: &Fallback, batched: bool) -> Vec<String> {
    let Fallback {
        reason,
        preferred,
        listing,
    } = fallback;
    let case = format!("{reason}, batched {batched}");
    let mut state = preferring(preferred);
    let mut failures: Vec<_> = deliver_listing(&mut state, listing, &case, batched)
        .into_iter()
        .collect();
    let first_visible = listing.iter().position(|entry| !entry.is_hidden);
    let landed = fill_and_focus(&state, 0);
    if landed
        != (
            first_visible.into_iter().collect(),
            Some((0, first_visible)),
        )
    {
        failures.push(format!(
            "{case}: selection and cursor {landed:?}, expected the first visible entry"
        ));
    }
    if state.reveal_pending(0)
        || state
            .take_unresolved_location_reveal(0, RequestId(1))
            .is_some()
    {
        failures.push(format!("{case}: the preferred entry became a reveal"));
    }
    failures
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

    assert_eq!(
        state.finish(RequestId(1), false, None, None),
        Some((0, false))
    );
    assert_eq!(state.columns[0].load_state, LoadState::Empty);
}

#[test]
fn depth_for_request_tracks_live_loads_only() {
    let depths = |state: &NavigationState, requests: [u64; 2]| {
        requests.map(|request| state.depth_for_request(RequestId(request)))
    };
    let mut state = NavigationState::default();
    state.navigate(location("/fixture"), RequestId(1));
    assert_eq!(depths(&state, [1, 9]), [Some(0), None]);

    assert!(state.descend(0, location("/fixture/sub"), RequestId(2)));
    assert_eq!(state.depth_for_request(RequestId(2)), Some(1));

    state.reload_column(0, RequestId(3));
    assert_eq!(depths(&state, [1, 3]), [None, Some(0)]);
}

#[test]
fn preferred_location_on_load_selects_the_child_when_it_arrives() {
    let mut failures = Vec::new();
    for delivery in [Delivery::Snapshot, Delivery::OneBatch, Delivery::LaterBatch] {
        let mut state = preferring("/docs/b2");
        failures.extend(deliver_docs(&mut state, delivery));
        let landed = fill_focus_and_anchor(&state);
        if landed != (vec![1], Some((0, Some(1))), Some(1)) {
            failures.push(format!(
                "{delivery:?}: selection, cursor and anchor {landed:?}; expected b2 (1)"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn preferred_location_falls_back_to_the_first_visible_entry() {
    let mut failures = Vec::new();
    for fallback in [
        Fallback {
            reason: "missing",
            preferred: "/docs/b2",
            listing: vec![docs("a1"), docs("c3")],
        },
        Fallback {
            reason: "hidden",
            preferred: "/docs/.b2",
            listing: vec![hidden_entry("/docs/.b2", ".b2"), docs("a1"), docs("c3")],
        },
    ] {
        for batched in [false, true] {
            failures.extend(fallback_failures(&fallback, batched));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
