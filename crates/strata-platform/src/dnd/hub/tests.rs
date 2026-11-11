//! A start that reaches no platform, for the headless tests of the source's guard, and the
//! hub's own tests.

use std::cell::Cell;
use std::sync::Arc;

use super::{Job, Posters, Release};

thread_local! {
    pub(crate) static FAKE_START: Cell<Option<Release>> = const { Cell::new(None) };
    /// What `still_holding` answers with no platform: whether the drag still runs.
    pub(crate) static FAKE_LIVE: Cell<Option<bool>> = const { Cell::new(None) };
    static NEXT: Cell<u64> = const { Cell::new(0) };
}

pub(super) fn next_id() -> u64 {
    NEXT.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

/// Review round 1: a window attached again (Rune recreates its native side on every Wayland
/// show) replaces its poster instead of adding one more.
#[test]
fn a_window_attached_again_keeps_one_poster() {
    let posters = Posters::default();
    let noop = || -> super::Poster { Arc::new(|_: Job| {}) };
    posters.set("main", noop());
    posters.set("second", noop());
    for _ in 0..10 {
        posters.set("main", noop());
    }
    assert_eq!(posters.len(), 2);
}
