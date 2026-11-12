// SPDX-License-Identifier: MIT

use std::{cell::OnceCell, cmp::Ordering, collections::HashSet};

use super::LocationSet;
use crate::model::{FileEntry, Location};

/// Selected locations and their cached rows, which the column forgets on every listing
/// change.
#[derive(Clone, Debug, Default)]
pub(super) struct SelectedLocations {
    set: LocationSet,
    // Ascending; exactly the rows whose location `set` contains.
    positions: OnceCell<Vec<usize>>,
}

impl From<HashSet<Location>> for SelectedLocations {
    fn from(locations: HashSet<Location>) -> Self {
        Self {
            set: LocationSet::Explicit(locations),
            positions: OnceCell::new(),
        }
    }
}

impl SelectedLocations {
    pub(super) fn all_visible(entries: &[FileEntry], show_hidden: bool) -> Self {
        Self {
            set: LocationSet::all_visible(entries, show_hidden),
            positions: OnceCell::new(),
        }
    }

    pub(super) fn is_all(&self) -> bool {
        matches!(self.set, LocationSet::All { .. })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.set.len()
    }

    pub(super) fn contains(&self, location: &Location) -> bool {
        self.set.contains(location)
    }

    pub(super) fn materialize(&self, entries: &[FileEntry]) -> HashSet<Location> {
        self.set.materialize(entries)
    }

    pub(super) fn equals_explicit(
        &self,
        locations: &HashSet<Location>,
        entries: &[FileEntry],
    ) -> bool {
        self.set.equals_explicit(locations, entries)
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn insert(&mut self, location: Location) -> bool {
        let inserted = self.set.insert(location);
        if inserted {
            self.forget_positions();
        }
        inserted
    }

    pub(super) fn remove(&mut self, location: &Location) -> bool {
        let removed = self.set.remove(location);
        if removed {
            self.forget_positions();
        }
        removed
    }

    pub(super) fn excludes_new(&mut self, location: Location) {
        self.forget_positions();
        self.set.excludes_new(location);
    }

    pub(super) fn relocate(&mut self, from: &Location, to: Location) {
        self.forget_positions();
        self.set.relocate(from, to);
    }

    pub(super) fn retain(&mut self, entries: &[FileEntry], hide_hidden: bool) {
        self.forget_positions();
        self.set.retain(entries, hide_hidden);
    }

    pub(super) fn forget_positions(&mut self) {
        self.positions.take();
    }

    pub(super) fn positions(
        &self,
        entries: &[FileEntry],
        cursor: Option<usize>,
        each_location_once: impl FnOnce() -> bool,
    ) -> &[usize] {
        if let Some(positions) = self.positions.get() {
            #[cfg(feature = "invariant-checks")]
            assert_eq!(
                positions,
                &self.scan(entries),
                "selection positions disagree with the listing"
            );
            return positions;
        }
        self.positions.get_or_init(|| {
            // A select-all's count can drift from its listing, so only an explicit
            // set may trust it.
            if let Some(cursor) = cursor.filter(|cursor| {
                matches!(self.set, LocationSet::Explicit(_))
                    && self.set.len() == 1
                    && entries
                        .get(*cursor)
                        .is_some_and(|entry| self.set.contains(&entry.location))
            }) && each_location_once()
            {
                return vec![cursor];
            }
            self.scan(entries)
        })
    }

    pub(super) fn scan(&self, entries: &[FileEntry]) -> Vec<usize> {
        if self.set.is_empty() {
            return Vec::new();
        }
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.set.contains(&entry.location))
            .map(|(position, _)| position)
            .collect()
    }

    /// The cached rows when they map one to one onto the selected locations. A lone
    /// location's cached row is its only one.
    pub(super) fn exact_positions(
        &self,
        each_location_once: impl FnOnce() -> bool,
    ) -> Option<&[usize]> {
        self.positions
            .get()
            .filter(|positions| {
                positions.len() == self.set.len() && (positions.len() <= 1 || each_location_once())
            })
            .map(Vec::as_slice)
    }

    pub(super) fn lone_position(&self) -> Option<usize> {
        match (&self.set, self.positions.get()?.as_slice()) {
            (LocationSet::Explicit(locations), &[position]) if locations.len() == 1 => {
                Some(position)
            }
            _ => None,
        }
    }

    /// Only after a listing change that added no row for the lone selected location.
    pub(super) fn move_lone_position(&mut self, position: usize) {
        self.positions = OnceCell::from(vec![position]);
    }

    /// `positions` must be ascending, distinct and within `entries`. An explicit
    /// selection changes by difference, so a drag rehashes only the rows that changed;
    /// a select-all becomes explicit on its first change.
    pub(super) fn select_positions(
        &mut self,
        entries: &[FileEntry],
        positions: Vec<usize>,
        each_location_once: impl Fn() -> bool,
    ) {
        debug_assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        let difference = self
            .exact_positions(&each_location_once)
            .map(|previous| sorted_difference(previous, &positions))
            .filter(|(removed, added)| removed.len() + added.len() < positions.len());
        self.forget_positions();
        match (difference, &mut self.set) {
            (Some((removed, added)), LocationSet::Explicit(locations)) => {
                for position in removed {
                    locations.remove(&entries[position].location);
                }
                locations.reserve(added.len());
                for position in added {
                    locations.insert(entries[position].location.clone());
                }
            }
            _ => {
                self.set = LocationSet::Explicit(
                    positions
                        .iter()
                        .map(|position| entries[*position].location.clone())
                        .collect(),
                );
            }
        }
        self.remember(positions, each_location_once);
    }

    /// `positions` are the rows the selection was built from or matched. Another row
    /// can hold a selected location only in a listing that repeats one.
    pub(super) fn remember(
        &mut self,
        positions: Vec<usize>,
        each_location_once: impl FnOnce() -> bool,
    ) {
        if positions.len() == self.set.len() && each_location_once() {
            self.positions = OnceCell::from(positions);
        }
    }
}

/// Positions only in `previous`, then positions only in `next`; both ascending.
fn sorted_difference(previous: &[usize], next: &[usize]) -> (Vec<usize>, Vec<usize>) {
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let (mut old, mut new) = (0, 0);
    loop {
        match (previous.get(old), next.get(new)) {
            (Some(left), Some(right)) => match left.cmp(right) {
                Ordering::Equal => {
                    old += 1;
                    new += 1;
                }
                Ordering::Less => {
                    removed.push(*left);
                    old += 1;
                }
                Ordering::Greater => {
                    added.push(*right);
                    new += 1;
                }
            },
            (Some(_), None) => {
                removed.extend_from_slice(&previous[old..]);
                return (removed, added);
            }
            (None, _) => {
                added.extend_from_slice(&next[new..]);
                return (removed, added);
            }
        }
    }
}
