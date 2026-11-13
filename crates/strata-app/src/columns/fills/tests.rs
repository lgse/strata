// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::{Column, ColumnFills, SETTLE};

fn first(fills: &mut ColumnFills) -> &mut Column {
    &mut fills.columns[0]
}

/// A column's fills wait while its range moves, as GTK's Columns rows ask for none while their
/// column scrolled in the last 150 ms: a first range or one that only grows as rows arrive is
/// no scroll; a moved start is, until it has been still for [`SETTLE`].
#[test]
fn a_scrolling_column_is_asked_for_once_it_settles() {
    let mut fills = ColumnFills::new(true);
    fills.fit(1);

    fills.see(0, 0..10);
    assert_eq!(first(&mut fills).settled(0.0), Ok(()), "a first range");
    fills.see(0, 0..20);
    assert_eq!(first(&mut fills).settled(0.01), Ok(()), "rows arriving");

    fills.see(0, 5..25);
    let waiting = first(&mut fills).settled(1.0);
    assert!(waiting.is_err(), "scrolled just now: {waiting:?}");
    assert!(
        first(&mut fills).settled(1.0 + SETTLE / 2.0).is_err(),
        "still settling"
    );
    assert_eq!(
        first(&mut fills).settled(1.0 + SETTLE + 0.001),
        Ok(()),
        "settled"
    );
}

/// Another folder at the same depth starts unscrolled: its first range is asked for at once,
/// however far down the last folder was, and a scroll still settling there is dropped.
#[test]
fn another_folder_in_a_column_is_asked_for_at_once() {
    let mut fills = ColumnFills::new(true);
    fills.fit(1);
    first(&mut fills).visit(&Location::local("/a"));
    fills.see(0, 0..10);
    fills.see(0, 40..50);
    assert!(first(&mut fills).settled(1.0).is_err(), "scrolled in /a");

    fills.see(0, 0..10);
    first(&mut fills).visit(&Location::local("/b"));
    assert_eq!(
        first(&mut fills).settled(1.01),
        Ok(()),
        "a first range in /b"
    );
}
