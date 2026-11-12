// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use crate::model::{FileEntry, Location};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LocationSet {
    Explicit(HashSet<Location>),
    All {
        excluded: HashSet<Location>,
        count: usize,
    },
}

impl Default for LocationSet {
    fn default() -> Self {
        Self::Explicit(HashSet::new())
    }
}

impl LocationSet {
    pub(super) fn all_visible(entries: &[FileEntry], show_hidden: bool) -> Self {
        let excluded = if show_hidden {
            HashSet::new()
        } else {
            entries
                .iter()
                .filter(|entry| entry.is_hidden)
                .map(|entry| entry.location.clone())
                .collect()
        };
        Self::All {
            count: entries.len().saturating_sub(excluded.len()),
            excluded,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Explicit(locations) => locations.len(),
            Self::All { count, .. } => *count,
        }
    }

    pub(super) fn contains(&self, location: &Location) -> bool {
        match self {
            Self::Explicit(locations) => locations.contains(location),
            Self::All { excluded, .. } => !excluded.contains(location),
        }
    }

    pub(super) fn insert(&mut self, location: Location) -> bool {
        match self {
            Self::Explicit(locations) => locations.insert(location),
            Self::All { excluded, count } => {
                step_count(count, excluded.remove(&location), usize::saturating_add)
            }
        }
    }

    pub(super) fn remove(&mut self, location: &Location) -> bool {
        match self {
            Self::Explicit(locations) => locations.remove(location),
            Self::All { excluded, count } => step_count(
                count,
                excluded.insert(location.clone()),
                usize::saturating_sub,
            ),
        }
    }

    pub(super) fn excludes_new(&mut self, location: Location) {
        if let Self::All { excluded, .. } = self {
            excluded.insert(location);
        }
    }

    pub(super) fn relocate(&mut self, from: &Location, to: Location) {
        match self {
            Self::Explicit(locations) => {
                if locations.remove(from) {
                    locations.insert(to);
                }
            }
            Self::All { excluded, .. } => {
                if excluded.remove(from) {
                    excluded.insert(to);
                }
            }
        }
    }

    pub(super) fn materialize(&self, entries: &[FileEntry]) -> HashSet<Location> {
        match self {
            Self::Explicit(locations) => locations.clone(),
            Self::All { excluded, .. } => entries
                .iter()
                .filter(|entry| !excluded.contains(&entry.location))
                .map(|entry| entry.location.clone())
                .collect(),
        }
    }

    pub(super) fn equals_explicit(
        &self,
        locations: &HashSet<Location>,
        entries: &[FileEntry],
    ) -> bool {
        self.len() == locations.len()
            && match self {
                Self::Explicit(selected) => selected == locations,
                Self::All { excluded, .. } => entries.iter().all(|entry| {
                    excluded.contains(&entry.location) != locations.contains(&entry.location)
                }),
            }
    }

    pub(super) fn retain(&mut self, entries: &[FileEntry], hide_hidden: bool) {
        match self {
            Self::Explicit(locations) => {
                let mut previous = std::mem::take(locations);
                *locations = entries
                    .iter()
                    .filter(|entry| !hide_hidden || !entry.is_hidden)
                    .filter_map(|entry| previous.take(&entry.location))
                    .collect();
            }
            Self::All { excluded, count } => {
                if hide_hidden {
                    excluded.extend(
                        entries
                            .iter()
                            .filter(|entry| entry.is_hidden)
                            .map(|entry| entry.location.clone()),
                    );
                }
                *count = entries
                    .iter()
                    .filter(|entry| !excluded.contains(&entry.location))
                    .count();
            }
        }
    }
}

/// Steps a full set's count by one when its exclusions `changed`.
fn step_count(count: &mut usize, changed: bool, step: fn(usize, usize) -> usize) -> bool {
    if changed {
        *count = step(*count, 1);
    }
    changed
}
