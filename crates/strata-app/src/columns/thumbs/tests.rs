// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::{Sync, Synced};

#[test]
fn a_poll_tells_the_store_what_changed_in_a_column() {
    let (a, b) = (Location::local("/a"), Location::local("/b"));
    let mut column = Synced::default();
    assert_eq!(column.next(&a, 1, 9, false), Sync::Again, "a new column");
    assert_eq!(
        column.next(&a, 1, 9, false),
        Sync::Nothing,
        "nothing changed"
    );
    assert_eq!(
        column.next(&a, 2, 9, false),
        Sync::Again,
        "its listing changed"
    );
    assert_eq!(
        column.next(&b, 2, 9, false),
        Sync::Again,
        "a sibling in its place, at the same listing version"
    );
    column.range = Some(3..9);
    assert_eq!(
        column.next(&b, 2, 9, true),
        Sync::View(3..9),
        "the rows shown win"
    );
    assert_eq!(column.next(&b, 2, 9, true), Sync::Step, "the step changed");
    assert_eq!(column.next(&b, 2, 9, false), Sync::Nothing);
    column.range = Some(0..0);
    assert_eq!(
        column.next(&b, 2, 9, false),
        Sync::Nothing,
        "no rows of a listed folder"
    );
    column.range = Some(0..0);
    assert_eq!(
        column.next(&b, 4, 9, false),
        Sync::Again,
        "no rows of a listed folder whose listing changed"
    );
    column.range = Some(0..0);
    assert_eq!(
        column.next(&b, 5, 0, false),
        Sync::View(0..0),
        "no rows of an empty folder"
    );
}

#[test]
fn held_thumb_clears_deferral_after_settle_and_redefers_on_scroll() {
    use crate::fills::SCROLL_SETTLE;

    let folder = Location::local("/photos");
    let mut column = Synced::default();
    column.see(0..20);
    assert_eq!(column.settled_for_drag(&folder, 1.0, false), Ok(false));

    // Active drag movement defers thumbnails and handle allocation.
    column.see(100..120);
    assert!(column.is_deferring(true));
    assert!(column.settled_for_drag(&folder, 1.05, true).is_err());
    assert!(column.is_deferring(true));

    // Holding the thumb still past SCROLL_SETTLE flushes and stops deferring handles while held.
    assert_eq!(
        column.settled_for_drag(&folder, 1.05 + SCROLL_SETTLE + 0.001, true),
        Ok(true)
    );
    assert!(
        !column.is_deferring(true),
        "settled held thumb stops skipping handles"
    );

    // Resuming movement while still holding re-enables deferral.
    column.see(200..220);
    assert!(column.is_deferring(true));
    assert!(column.settled_for_drag(&folder, 2.0, true).is_err());

    // Releasing flushes immediately and clears deferral.
    assert_eq!(column.settled_for_drag(&folder, 2.01, false), Ok(true));
    assert!(!column.is_deferring(false));
}
