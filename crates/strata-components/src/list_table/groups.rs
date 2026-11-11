//! File-type groups from a sorted listing: C1f's [`Groups`], one per run of entries that share a
//! group, in display order.

use std::collections::HashSet;
use std::sync::Arc;

use crate::collection::element::{Group, Groups};

/// The groups of `len` entries in display order, where `group_of(i)` names entry `i`'s group
/// by a key and its header's label. Each run of entries with one key makes a group, at
/// O(len) once per regroup. Give the result to `CollectionState::set_groups` before `enter` or
/// `sync` with the listing.
///
/// Which groups there are and their order (Strata's Folder first, then MIME descriptions A–Z
/// with Other last, the sort kept inside each) is the host's: it sorts so that each group's
/// entries are contiguous. A key that comes back after another group still makes a group of
/// its own, keyed by the next key no group has, so group keys stay unique.
///
/// Used by: hosts.
pub fn groups_from_runs(len: usize, mut group_of: impl FnMut(usize) -> (u64, Arc<str>)) -> Groups {
    let mut groups: Vec<Group> = Vec::new();
    let mut used: HashSet<u64> = HashSet::new();
    let mut current: Option<u64> = None;
    for i in 0..len {
        let (key, label) = group_of(i);
        match (current, groups.last_mut()) {
            (Some(k), Some(last)) if k == key => last.len += 1,
            _ => {
                // A key that comes back takes the next one no group has, so keys stay unique
                // whatever the host's keys are.
                let mut unique = key;
                while !used.insert(unique) {
                    unique = unique.wrapping_add(1);
                }
                current = Some(key);
                groups.push(Group::new(unique, label, 1));
            }
        }
    }
    Groups::new(groups)
}
