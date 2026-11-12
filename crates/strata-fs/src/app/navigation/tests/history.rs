// SPDX-License-Identifier: MIT

use super::*;

fn locations(paths: &[&str]) -> Vec<Location> {
    paths.iter().map(|path| location(path)).collect()
}

fn back_forward_and_parent(state: &NavigationState) -> (bool, bool, bool) {
    (
        state.can_go_back(),
        state.can_go_forward(),
        state.can_go_parent(),
    )
}

#[test]
fn navigation_availability_tracks_history_and_parent() {
    let mut state = NavigationState::default();
    assert_eq!(back_forward_and_parent(&state), (false, false, false));

    state.navigate(location("/"), RequestId(1));
    assert!(!state.can_go_parent());
    state.navigate(location("/home"), RequestId(2));
    assert_eq!(back_forward_and_parent(&state), (true, false, true));

    let back = state
        .go_back_to(1)
        .expect("back history should be available");
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
        .go_back_to(1)
        .expect("a committed path should be available");
    assert_eq!(back.locations(), &[location("/home")]);
    state.restore(back, [RequestId(3)]);

    let forward = state
        .go_forward()
        .expect("the descended path should be available");
    assert_eq!(forward.locations(), locations(&["/home", "/home/projects"]));
}

#[test]
fn back_to_skips_entries_and_keeps_them_reachable_forward() {
    let mut state = NavigationState::default();
    for (request, path) in ["/home", "/home/projects", "/tmp", "/var"]
        .into_iter()
        .enumerate()
    {
        state.navigate(location(path), RequestId(request as u64 + 1));
    }
    let full_history = locations(&["/tmp", "/home/projects", "/home"]);
    assert_eq!(state.back_locations(), full_history);

    assert!(state.go_back_to(0).is_none());
    assert!(state.go_back_to(4).is_none());
    let target = state.go_back_to(2).expect("two entries back");
    assert_eq!(target.locations(), &[location("/home/projects")]);
    state.restore(target, [RequestId(5)]);
    assert_eq!(state.back_locations(), vec![location("/home")]);

    let forward = state.go_forward().expect("the skipped entry comes first");
    assert_eq!(forward.locations(), &[location("/tmp")]);
    state.restore(forward, [RequestId(6)]);
    let forward = state.go_forward().expect("then the entry that was current");
    assert_eq!(forward.locations(), &[location("/var")]);
    state.restore(forward, [RequestId(7)]);
    assert!(!state.can_go_forward());
    assert_eq!(state.back_locations(), full_history);
}

#[test]
fn parent_removes_the_deepest_committed_column() {
    let mut state = NavigationState::default();
    state.navigate(location("/home"), RequestId(1));
    assert!(state.descend(0, location("/home/projects"), RequestId(2)));

    let parent = state.go_parent().expect("the path has a parent");
    assert_eq!(parent.locations(), &[location("/home")]);
}
