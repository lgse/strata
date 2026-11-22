// SPDX-License-Identifier: MIT

//! Shared track key and bounded LRU cache for media preview background decodes
//! (waveform peaks and video storyboards), matching GTK's `TrackKey` and `Lru`
//! (`src/ui/preview/audio/details.rs`).

use std::collections::VecDeque;

use strata_core::model::{FileEntry, MetadataValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrackKey {
    location: String,
    size: Option<u64>,
    modified: Option<i64>,
}

impl TrackKey {
    pub(super) fn of(entry: &FileEntry) -> Self {
        Self {
            location: entry.location.display_path(),
            size: known(&entry.size),
            modified: known(&entry.modified_unix_seconds),
        }
    }
}

fn known<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    }
}

pub(super) struct Lru<K, V> {
    capacity: usize,
    entries: VecDeque<(K, V)>,
}

impl<K: PartialEq, V: Clone> Lru<K, V> {
    pub(super) const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    pub(super) fn get(&mut self, key: &K) -> Option<V> {
        let index = self.entries.iter().position(|(entry, _)| entry == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    pub(super) fn insert(&mut self, key: K, value: V) {
        self.entries.retain(|(entry, _)| *entry != key);
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }

    #[cfg(test)]
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}
