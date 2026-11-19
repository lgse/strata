// SPDX-License-Identifier: MIT

//! Bulk location matching under GIO equivalence, linear in the number of locations.

use crate::adapters::gio_file_for_location;
use crate::model::Location;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

type FilesByUri = HashMap<glib::GString, Vec<gio::File>>;

/// Equal `GFile`s always have the same URI, so bucketing by URI and confirming
/// with `GFile::equal` gives the same answers as comparing every pair.
fn files_by_uri<'a>(locations: impl IntoIterator<Item = &'a Location>) -> FilesByUri {
    let mut buckets = FilesByUri::new();
    for location in locations {
        let file = gio_file_for_location(location);
        buckets.entry(file.uri()).or_default().push(file);
    }
    buckets
}

fn take_equal(buckets: &mut FilesByUri, location: &Location) -> bool {
    let file = gio_file_for_location(location);
    let Some(bucket) = buckets.get_mut(&file.uri()) else {
        return false;
    };
    let Some(index) = bucket.iter().position(|candidate| candidate.equal(&file)) else {
        return false;
    };
    bucket.swap_remove(index);
    true
}

pub(super) struct LocationSet<'a> {
    locations: &'a [Location],
    exact: HashSet<&'a Location>,
    files: OnceCell<FilesByUri>,
}

impl<'a> LocationSet<'a> {
    pub(super) fn new(locations: &'a [Location]) -> Self {
        Self {
            locations,
            exact: locations.iter().collect(),
            files: OnceCell::new(),
        }
    }

    pub(super) fn contains(&self, location: &Location) -> bool {
        if self.exact.contains(location) {
            return true;
        }
        let file = gio_file_for_location(location);
        self.files
            .get_or_init(|| files_by_uri(self.locations))
            .get(&file.uri())
            .is_some_and(|bucket| bucket.iter().any(|candidate| candidate.equal(&file)))
    }
}

pub(super) fn same_locations(left: &[Location], right: &[Location]) -> bool {
    if left.is_empty() || left.len() != right.len() {
        return false;
    }
    let left_set: HashSet<_> = left.iter().collect();
    let right_set: HashSet<_> = right.iter().collect();
    if left_set.len() == right_set.len() && left_set == right_set {
        return true;
    }
    let mut unmatched = files_by_uri(right);
    left.iter()
        .all(|location| take_equal(&mut unmatched, location))
}
