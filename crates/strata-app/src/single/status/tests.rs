// SPDX-License-Identifier: MIT

//! What a pane that shows no entry says, on the poll's clock.

use strata_core::model::Location;

use super::{GRACE, Surface, reading};
use crate::listing::{Column, Edit, Load};

fn read(surface: &Surface, listing: &Column) -> Option<(String, bool)> {
    surface
        .status(listing)
        .map(|(said, drawn)| (said.to_owned(), drawn))
}

/// A folder being read says nothing for GTK's grace period, then "Loading", and nothing is
/// drawn either way; once read empty, it says so and draws it.
#[test]
fn a_read_is_announced_only_once_it_outlasts_the_grace_period() {
    let mut surface = Surface::translated();
    let mut tx = rune::Tx::default();
    let mut listing = Column::new(Location::local("/folder"), false, None);
    assert!(reading(&listing));
    let grace = GRACE.as_secs_f64();

    assert!(!surface.watch(Some(&listing), 10.0, &mut tx));
    assert_eq!(read(&surface, &listing), Some((String::new(), false)));
    assert!(!surface.watch(Some(&listing), 10.0 + grace / 2.0, &mut tx));
    assert_eq!(read(&surface, &listing), Some((String::new(), false)));

    assert!(surface.watch(Some(&listing), 10.0 + grace, &mut tx));
    assert_eq!(
        read(&surface, &listing),
        Some(("Loading".to_owned(), false))
    );

    listing.set_load(Load::Listed);
    listing.apply(Edit::Replace(Vec::new()));
    assert!(surface.watch(Some(&listing), 11.0, &mut tx));
    let empty = ("This directory is empty".to_owned(), true);
    assert_eq!(read(&surface, &listing), Some(empty));
}

/// A read that ends within the grace period is never announced, and the next folder's read
/// waits out a grace period of its own.
#[test]
fn a_quick_read_is_never_announced_and_each_folder_waits_its_own() {
    let mut surface = Surface::translated();
    let mut tx = rune::Tx::default();
    let grace = GRACE.as_secs_f64();
    let mut first = Column::new(Location::local("/first"), false, None);
    surface.watch(Some(&first), 1.0, &mut tx);
    first.set_load(Load::Listed);
    assert!(!surface.watch(Some(&first), 1.0 + grace / 2.0, &mut tx));

    let second = Column::new(Location::local("/second"), false, None);
    surface.watch(Some(&second), 2.0, &mut tx);
    assert!(!surface.watch(Some(&second), 2.0 + grace / 2.0, &mut tx));
    assert_eq!(read(&surface, &second), Some((String::new(), false)));
    assert!(surface.watch(Some(&second), 2.0 + grace, &mut tx));
}
