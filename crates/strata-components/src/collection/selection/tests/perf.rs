//! Perf: 100,000 entries, select all, then a Shift range across the list, each under a
//! millisecond. Release builds only:
//! `cargo test --release -p strata-components --lib selection::tests::perf -- --nocapture`.
//!
//! Each timing is the best of five runs, each on a fresh selection, so allocation counts. Each
//! timed block also reads the count and 60 rows, as a frame would, so no cost hides in a lazy
//! first read.

use std::time::{Duration, Instant};

use super::*;

const N: u64 = 100_000;
const BUDGET: Duration = Duration::from_millis(1);
const RUNS: usize = 5;

/// 100,000 entries with sparse ids, shuffled, so neither hashing nor the source's order is
/// kind to the selection.
fn big_listing() -> ListSource {
    let mut ids: Vec<u64> = (0..N).map(|i| i * 7_919 + 13).collect();
    let mut x = 0x2545_F491_4F6C_DD1D_u64;
    for i in (1..ids.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        ids.swap(i, (x % (i as u64 + 1)) as usize);
    }
    ListSource::new(ids.into_iter().map(entry).collect())
}

/// What a frame reads after the change.
fn read(sel: &Selection, s: &ListSource) -> usize {
    let window = (49_970..50_030)
        .filter(|&i| sel.is_selected_at(s, i))
        .count();
    sel.len() + window
}

/// The best of [`RUNS`] timings of `op`, each on the selection `setup` makes.
fn best(
    s: &ListSource,
    setup: impl Fn() -> Selection,
    op: impl Fn(&mut Selection) -> Changed,
) -> (Duration, Selection) {
    let mut best = Duration::MAX;
    let mut last = Selection::new();
    for _ in 0..RUNS {
        let mut sel = setup();
        let t = Instant::now();
        let changed = op(&mut sel);
        std::hint::black_box((changed, read(&sel, s)));
        best = best.min(t.elapsed());
        last = sel;
    }
    (best, last)
}

#[test]
#[cfg_attr(debug_assertions, ignore = "perf: run with --release")]
fn perf_select_all_then_shift_range_100k() {
    let s = big_listing();
    let n = s.len();

    let (select_all, sel) = best(&s, Selection::new, |sel| sel.select_all(&s));
    assert_eq!(sel.len(), n);

    // Shift+End from the first entry, after select-all: the range spans the whole list.
    let after_select_all = || {
        let mut sel = Selection::new();
        sel.select_all(&s);
        sel
    };
    let (shift_across, sel) = best(&s, after_select_all, |sel| {
        sel.extend_to(&s, n - 1, Extend::Replace)
    });
    assert_eq!(sel.len(), n);
    assert_eq!(sel.anchor_index(&s), Some(0));

    // A Shift range from the middle to the start, from a single selection.
    let from_middle = || {
        let mut sel = Selection::new();
        sel.select_only(&s, n / 2);
        sel
    };
    let (shift_half, sel) = best(&s, from_middle, |sel| sel.extend_to(&s, 0, Extend::Replace));
    assert_eq!(sel.len(), n / 2 + 1);

    // Off the bar, for the record: the costs that hash every id once.
    let (toggle, _) = best(&s, after_select_all, |sel| sel.toggle(&s, 7));
    let (sync, _) = best(&s, after_select_all, |sel| {
        sel.sync(&s, SyncRule::default())
    });

    eprintln!(
        "selection perf, {n} entries (best of {RUNS}):\n  select_all        {select_all:?}\n  \
         shift across all  {shift_across:?}\n  shift half        {shift_half:?}\n  \
         toggle after all  {toggle:?} (not on the bar)\n  sync after all    {sync:?} (not on the bar)"
    );
    for (what, took) in [
        ("select_all", select_all),
        ("Shift range across the list", shift_across),
        ("Shift range across half the list", shift_half),
    ] {
        assert!(took < BUDGET, "{what} took {took:?}, over {BUDGET:?}");
    }
}
