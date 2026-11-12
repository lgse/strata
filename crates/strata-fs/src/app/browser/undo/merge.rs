// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};

use crate::{
    model::Location,
    services::{PlacedSnapshot, RenamedItem},
};

/// Merge bookkeeping collected while a paste runs: which sources merged, and
/// what each merge created or overwrote. Sources matter for move transfers,
/// where a merged source must not join the move-undo records.
#[derive(Default)]
pub(in crate::app::browser) struct MergeUndoState {
    pub(in crate::app::browser) sources: HashSet<Location>,
    pub(in crate::app::browser) created: Vec<Location>,
    /// `None` once its incoming copy was saved under another name.
    pub(in crate::app::browser) overwritten: Vec<Option<Location>>,
    pub(in crate::app::browser) created_at: HashMap<Location, Vec<usize>>,
    pub(in crate::app::browser) overwritten_at: HashMap<Location, Vec<usize>>,
    pub(in crate::app::browser) placed: HashMap<Location, PlacedSnapshot>,
}

impl MergeUndoState {
    pub(in crate::app::browser) fn merged(
        &mut self,
        source: Location,
        created: Vec<Location>,
        overwritten: Vec<Location>,
        placed: HashMap<Location, PlacedSnapshot>,
    ) {
        self.sources.insert(source);
        for location in created {
            self.create(location);
        }
        for location in overwritten {
            self.overwritten_at
                .entry(location.clone())
                .or_default()
                .push(self.overwritten.len());
            self.overwritten.push(Some(location));
        }
        self.placed.extend(placed);
    }

    pub(in crate::app::browser) fn create(&mut self, location: Location) {
        self.created_at
            .entry(location.clone())
            .or_default()
            .push(self.created.len());
        self.created.push(location);
    }

    /// An overwritten original stays in Trash when its copy landed under
    /// another name, since another item now holds that name. Renames apply in
    /// reported order and are not chained: a record under a planned name
    /// follows only the rename that starts at that name.
    pub(in crate::app::browser) fn rename(&mut self, renamed: &[RenamedItem]) {
        for item in renamed {
            if let Some(positions) = self.created_at.remove(&item.requested) {
                for &position in &positions {
                    self.created[position] = item.saved_as.clone();
                }
                self.created_at
                    .entry(item.saved_as.clone())
                    .or_default()
                    .extend(positions);
            }
            let first = self
                .overwritten_at
                .get_mut(&item.requested)
                .filter(|positions| !positions.is_empty())
                .map(|positions| positions.remove(0));
            if let Some(position) = first {
                self.overwritten[position] = None;
                self.create(item.saved_as.clone());
            }
        }
    }

    pub(in crate::app::browser) fn into_records(
        self,
    ) -> (
        Vec<Location>,
        Vec<Location>,
        HashMap<Location, PlacedSnapshot>,
    ) {
        let overwritten = self.overwritten.into_iter().flatten().collect();
        (self.created, overwritten, self.placed)
    }
}

pub(in crate::app::browser) fn retain_merge_records(
    placed: &mut HashMap<Location, PlacedSnapshot>,
    created: &[Location],
    overwritten: &[Location],
) {
    let undone: HashSet<&Location> = created.iter().chain(overwritten).collect();
    placed.retain(|location, _| undone.contains(location));
}
