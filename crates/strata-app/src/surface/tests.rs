// SPDX-License-Identifier: MIT

//! What a listing that shows no entry says, on the poll's clock.

use strata_core::model::Location;

use super::{GRACE, Surface, reading};
use crate::listing::{Column, Edit, Load};

fn read(surface: &Surface, place: usize, listing: &Column) -> Option<(String, bool)> {
    surface
        .status(place, listing)
        .map(|(said, drawn)| (said.to_owned(), drawn))
}

fn folder(path: &str) -> Column {
    Column::new(Location::local(path), false, None)
}

fn silent() -> Option<(String, bool)> {
    Some((String::new(), false))
}

fn one(listing: &Column) -> impl ExactSizeIterator<Item = Option<&Column>> {
    [Some(listing)].into_iter()
}

fn loading() -> Option<(String, bool)> {
    Some(("Loading".to_owned(), false))
}

/// A folder being read says nothing for GTK's grace period, then "Loading", and nothing is
/// drawn either way; once read empty, it says so and draws it. The poll asks the view to run
/// for the announcement only, which the clock alone brings.
#[test]
fn a_read_is_announced_only_once_it_outlasts_the_grace_period() {
    let mut surface = Surface::translated();
    let mut tx = rune::Tx::default();
    let mut listing = folder("/folder");
    assert!(reading(&listing));
    let grace = GRACE.as_secs_f64();

    assert!(!surface.watch(one(&listing), 10.0, &mut tx));
    assert_eq!(read(&surface, 0, &listing), silent());
    assert!(!surface.watch(one(&listing), 10.0 + grace / 2.0, &mut tx));
    assert_eq!(read(&surface, 0, &listing), silent());

    assert!(surface.watch(one(&listing), 10.0 + grace, &mut tx));
    assert_eq!(read(&surface, 0, &listing), loading());

    listing.set_load(Load::Listed);
    listing.apply(Edit::Replace(Vec::new()));
    // The change that ended the read runs the view; the poll asks for nothing more.
    assert!(!surface.watch(one(&listing), 11.0, &mut tx));
    let empty = ("This directory is empty".to_owned(), true);
    assert_eq!(read(&surface, 0, &listing), Some(empty));
}

/// A read that ends within the grace period is never announced, and the next folder's read
/// waits out a grace period of its own.
#[test]
fn a_quick_read_is_never_announced_and_each_folder_waits_its_own() {
    let mut surface = Surface::translated();
    let mut tx = rune::Tx::default();
    let grace = GRACE.as_secs_f64();
    let mut first = folder("/first");
    surface.watch(one(&first), 1.0, &mut tx);
    first.set_load(Load::Listed);
    assert!(!surface.watch(one(&first), 1.0 + grace / 2.0, &mut tx));

    let second = folder("/second");
    surface.watch(one(&second), 2.0, &mut tx);
    assert!(!surface.watch(one(&second), 2.0 + grace / 2.0, &mut tx));
    assert_eq!(read(&surface, 0, &second), silent());
    assert!(surface.watch(one(&second), 2.0 + grace, &mut tx));
}

/// Columns each wait out their own read: a column opened later is still quiet when an
/// earlier one is announced, and a column the view no longer has leaves nothing behind.
#[test]
fn each_column_waits_out_its_own_read_and_a_closed_one_is_forgotten() {
    let mut surface = Surface::translated();
    let mut tx = rune::Tx::default();
    let grace = GRACE.as_secs_f64();
    let (root, child) = (folder("/root"), folder("/root/child"));
    surface.watch([Some(&root)].into_iter(), 1.0, &mut tx);
    let both = || [Some(&root), Some(&child)].into_iter();
    surface.watch(both(), 1.0 + grace / 2.0, &mut tx);

    assert!(surface.watch(both(), 1.0 + grace, &mut tx));
    assert_eq!(read(&surface, 0, &root), loading());
    assert_eq!(read(&surface, 1, &child), silent());

    assert!(!surface.watch([Some(&root)].into_iter(), 1.0 + grace * 2.0, &mut tx));
    assert_eq!(surface.readings.len(), 1);
}

/// A folder of the same name in another place, opened where the first was, waits out a
/// grace period of its own: the reading is the folder's, not its name's.
#[test]
fn another_folder_of_the_same_name_in_the_same_place_restarts_the_grace_period() {
    let mut surface = Surface::translated();
    let mut tx = rune::Tx::default();
    let grace = GRACE.as_secs_f64();
    let first = folder("/a/docs");
    surface.watch(one(&first), 1.0, &mut tx);
    assert!(surface.watch(one(&first), 1.0 + grace, &mut tx));
    assert_eq!(read(&surface, 0, &first), loading());

    let second = folder("/b/docs");
    assert_eq!(first.title(), second.title());
    assert!(!surface.watch(one(&second), 1.0 + grace * 1.5, &mut tx));
    assert_eq!(read(&surface, 0, &second), silent());
    assert!(surface.watch(one(&second), 1.0 + grace * 2.5, &mut tx));
    assert_eq!(read(&surface, 0, &second), loading());
}
