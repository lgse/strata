// SPDX-License-Identifier: MIT

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
    max_bytes: usize,
    bytes: usize,
    entries: VecDeque<(K, V, usize)>,
}

impl<K: PartialEq, V: Clone> Lru<K, V> {
    pub(super) const fn new(capacity: usize) -> Self {
        Self::with_byte_limit(capacity, usize::MAX)
    }

    pub(super) const fn with_byte_limit(capacity: usize, max_bytes: usize) -> Self {
        Self {
            capacity,
            max_bytes,
            bytes: 0,
            entries: VecDeque::new(),
        }
    }

    pub(super) fn get(&mut self, key: &K) -> Option<V> {
        let index = self.entries.iter().position(|(entry, _, _)| entry == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    pub(super) fn insert(&mut self, key: K, value: V) {
        self.insert_sized(key, value, 0);
    }

    pub(super) fn insert_sized(&mut self, key: K, value: V, size: usize) {
        if let Some(index) = self.entries.iter().position(|(entry, _, _)| *entry == key)
            && let Some((_, _, old_size)) = self.entries.remove(index)
        {
            self.bytes = self.bytes.saturating_sub(old_size);
        }
        if size > self.max_bytes {
            return;
        }
        while self.entries.len() >= self.capacity
            || self.bytes.saturating_add(size) > self.max_bytes
        {
            let Some((_, _, evicted_size)) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(evicted_size);
        }
        self.bytes = self.bytes.saturating_add(size);
        self.entries.push_back((key, value, size));
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.entries.shrink_to_fit();
    }

    #[cfg(test)]
    pub(super) fn byte_count(&self) -> usize {
        self.bytes
    }
}
