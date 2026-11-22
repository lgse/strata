// SPDX-License-Identifier: MIT

//! Incremental file-type group buckets for a grouped [`super::Column`].
//!
//! Staged publication appends rows in browser order. Keeping one bucket per [`TypeGroup`] in
//! `compare_type_groups` order avoids re-scanning and re-classifying already-indexed rows on
//! every chunk, and exposes O(groups) run lengths for [`crate::pane`].

use std::collections::HashMap;

use strata_core::model::ViewPreferences;
use strata_fs::app::compare_type_groups;

use super::TypeGroup;

#[derive(Default)]
pub(super) struct GroupedBuckets {
    /// Buckets ordered by `compare_type_groups`, each holding browser row positions in
    /// ascending browser order.
    buckets: Vec<(TypeGroup, Vec<usize>)>,
    /// Maps each seen [`TypeGroup`] to its index in `buckets`.
    bucket_of: HashMap<TypeGroup, usize>,
    /// Cached `(group, len)` runs for non-empty buckets, matching `shown`'s order.
    runs: Vec<(TypeGroup, usize)>,
}

impl GroupedBuckets {
    pub(super) fn clear(&mut self) {
        self.buckets.clear();
        self.bucket_of.clear();
        self.runs.clear();
    }

    /// Rebuilds all buckets from `positions` (browser positions in ascending browser order)
    /// and their corresponding [`TypeGroup`]s.
    pub(super) fn rebuild(
        &mut self,
        preferences: ViewPreferences,
        items: impl Iterator<Item = (usize, TypeGroup)>,
    ) {
        self.clear();
        for (position, group) in items {
            self.push_unsorted(position, group);
        }
        self.sort_buckets(preferences);
        self.refresh_runs();
    }

    /// Appends a single visible row (at a browser position greater than all previous rows)
    /// into its group bucket, inserting a new bucket in sorted order if unseen.
    pub(super) fn push(&mut self, position: usize, group: TypeGroup, preferences: ViewPreferences) {
        if let Some(&idx) = self.bucket_of.get(&group) {
            self.buckets[idx].1.push(position);
            return;
        }
        let desc = group.description();
        let insert_at = self.buckets.partition_point(|(existing, _)| {
            compare_type_groups(&existing.description(), &desc, preferences).is_lt()
        });
        self.buckets.insert(insert_at, (group, vec![position]));
        self.reindex_lookup();
    }

    /// Rebuilds the cached `(group, len)` summary after one or more appends.
    pub(super) fn refresh_runs(&mut self) {
        self.runs.clear();
        self.runs.extend(
            self.buckets
                .iter()
                .filter(|(_, rows)| !rows.is_empty())
                .map(|(group, rows)| (*group, rows.len())),
        );
    }

    /// Writes the flattened grouped browser positions into `shown` and updates `back`.
    pub(super) fn flatten_into(
        &self,
        total_rows: usize,
        shown: &mut Vec<usize>,
        back: &mut Vec<usize>,
    ) {
        shown.clear();
        let total_shown: usize = self.buckets.iter().map(|(_, rows)| rows.len()).sum();
        shown.reserve(total_shown);
        for (_, rows) in &self.buckets {
            shown.extend_from_slice(rows);
        }
        back.clear();
        back.resize(total_rows, usize::MAX);
        for (index, &position) in shown.iter().enumerate() {
            back[position] = index;
        }
    }

    /// Fast path when only a single group exists: appends new positions directly to `shown`
    /// and `back` in O(appended) without rewriting earlier entries.
    pub(super) fn single_group(&self) -> bool {
        self.buckets.len() <= 1
    }

    pub(super) fn runs(&self) -> &[(TypeGroup, usize)] {
        &self.runs
    }

    fn push_unsorted(&mut self, position: usize, group: TypeGroup) {
        if let Some(&idx) = self.bucket_of.get(&group) {
            self.buckets[idx].1.push(position);
        } else {
            let idx = self.buckets.len();
            self.buckets.push((group, vec![position]));
            self.bucket_of.insert(group, idx);
        }
    }

    fn sort_buckets(&mut self, preferences: ViewPreferences) {
        if self.buckets.len() <= 1 {
            return;
        }
        let names: Vec<_> = self
            .buckets
            .iter()
            .map(|(group, _)| group.description())
            .collect();
        let mut order: Vec<usize> = (0..self.buckets.len()).collect();
        order.sort_by(|&a, &b| compare_type_groups(&names[a], &names[b], preferences));
        let mut slots: Vec<Option<(TypeGroup, Vec<usize>)>> =
            self.buckets.drain(..).map(Some).collect();
        self.buckets = order
            .into_iter()
            .map(|i| slots[i].take().expect("unique bucket index"))
            .collect();
        self.reindex_lookup();
    }

    fn reindex_lookup(&mut self) {
        self.bucket_of.clear();
        for (idx, (group, _)) in self.buckets.iter().enumerate() {
            self.bucket_of.insert(*group, idx);
        }
    }
}
