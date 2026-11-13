// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::{Sync, Synced};

/// What a poll tells the store of a column: the rows its view reported, else the step that
/// changed, else its last rows again when its listing changed or another folder took it. No
/// rows reported of a listing that has entries is Rune's report of a list at no height, not
/// rows shown.
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
        column.next(&b, 3, 0, false),
        Sync::View(0..0),
        "no rows of an empty folder"
    );
}
