// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::ColumnFills;
use crate::fills::{SCROLL_SETTLE as SETTLE, ScrollSettle};

fn first(fills: &mut ColumnFills) -> &mut ScrollSettle {
    fills.columns[0].settle_mut()
}

#[test]
fn a_scrolling_column_is_asked_for_once_it_settles() {
    let mut fills = ColumnFills::new(true);
    fills.fit(1);

    fills.see(0, 0..10);
    assert_eq!(
        first(&mut fills).settled(0.0, false),
        Ok(()),
        "a first range"
    );
    fills.see(0, 0..20);
    assert_eq!(
        first(&mut fills).settled(0.01, false),
        Ok(()),
        "rows arriving"
    );

    fills.see(0, 5..25);
    let waiting = first(&mut fills).settled(1.0, false);
    assert!(waiting.is_err(), "scrolled just now: {waiting:?}");
    assert!(
        first(&mut fills)
            .settled(1.0 + SETTLE / 2.0, false)
            .is_err(),
        "still settling"
    );
    assert_eq!(
        first(&mut fills).settled(1.0 + SETTLE + 0.001, false),
        Ok(()),
        "settled"
    );
}

#[test]
fn another_folder_in_a_column_is_asked_for_at_once() {
    let mut fills = ColumnFills::new(true);
    fills.fit(1);
    let scrolled_in = |fills: &mut ColumnFills, folder: &str, clock: f64| {
        first(fills).visit(&Location::local(folder));
        fills.see(0, 0..10);
        fills.see(0, 40..50);
        assert!(
            first(fills).settled(clock, false).is_err(),
            "scrolled in {folder}"
        );
    };

    scrolled_in(&mut fills, "/a", 1.0);
    fills.see(0, 0..10);
    first(&mut fills).visit(&Location::local("/b"));
    assert_eq!(
        first(&mut fills).settled(1.01, false),
        Ok(()),
        "range, then /b"
    );

    scrolled_in(&mut fills, "/c", 2.0);
    first(&mut fills).visit(&Location::local("/d"));
    fills.see(0, 0..10);
    assert_eq!(
        first(&mut fills).settled(2.01, false),
        Ok(()),
        "/d, then its range"
    );
}
