// SPDX-License-Identifier: MIT

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use crate::services::ItemIdentity;

/// Keys kept for Put back or undo; the oldest items go first.
const TRASHED_KEYS_LIMIT: usize = 10_000;

/// Values of trashed items, kept in memory for this session only.
pub(super) struct TrashedValues<T> {
    items: VecDeque<TrashedItem<T>>,
    keys: usize,
}

struct TrashedItem<T> {
    root: PathBuf,
    identity: ItemIdentity,
    values: T,
    keys: usize,
}

impl<T> Default for TrashedValues<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
            keys: 0,
        }
    }
}

impl<T> TrashedValues<T> {
    /// An older entry proven to be the same item belongs to one that left Trash
    /// and came back; a reused inode number alone keeps both.
    pub(super) fn forget_identity(&mut self, identity: ItemIdentity) {
        let keys = &mut self.keys;
        self.items.retain(|item| {
            let (kept, now) = (item.identity.evidence, identity.evidence);
            let reused = (kept.device, kept.inode) == (now.device, now.inode)
                && item.identity.unchanged(&identity);
            if reused {
                *keys -= item.keys;
            }
            !reused
        });
    }

    /// `keys` counts the values towards the limit. Callers forget `identity`
    /// first, whether or not they keep anything for it.
    pub(super) fn keep(&mut self, root: PathBuf, identity: ItemIdentity, values: T, keys: usize) {
        self.keys += keys;
        self.items.push_back(TrashedItem {
            root,
            identity,
            values,
            keys,
        });
        while self.keys > TRASHED_KEYS_LIMIT && self.items.len() > 1 {
            if let Some(oldest) = self.items.pop_front() {
                self.keys -= oldest.keys;
            }
        }
    }

    /// What was kept for the very item now back at `root`; another item
    /// restored there gets nothing.
    pub(super) fn take_restored(&mut self, root: &Path, identity: ItemIdentity) -> Option<T> {
        let index = self
            .items
            .iter()
            .position(|item| item.root == root && item.identity.unchanged(&identity))?;
        let item = self.items.remove(index)?;
        self.keys -= item.keys;
        Some(item.values)
    }
}
