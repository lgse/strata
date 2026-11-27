// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::sync::Arc;

use crate::collection::element::{Group, Groups};

/// The groups of `len` entries in display order.
pub fn groups_from_runs(len: usize, mut group_of: impl FnMut(usize) -> (u64, Arc<str>)) -> Groups {
    let mut groups: Vec<Group> = Vec::new();
    let mut used: HashSet<u64> = HashSet::new();
    let mut current: Option<u64> = None;
    for i in 0..len {
        let (key, label) = group_of(i);
        match (current, groups.last_mut()) {
            (Some(k), Some(last)) if k == key => last.len += 1,
            _ => {
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
