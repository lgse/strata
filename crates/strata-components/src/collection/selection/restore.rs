// SPDX-License-Identifier: MIT

use std::collections::HashMap;
use std::hash::Hash;

use crate::contract::EntryId;

#[cfg(test)]
mod tests;

/// One folder's remembered position: `Selection::position` when it was left, plus the element's scroll position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Position {
    /// The cursor's entry.
    pub cursor: Option<EntryId>,
    /// Where the cursor was, for the neighbour rule if its entry is gone on return.
    pub cursor_index: Option<usize>,
    /// The anchor's entry.
    pub anchor: Option<EntryId>,
    /// The selected entries, in the order they were shown.
    pub selected: Vec<EntryId>,
    /// Whether the selection was a load's automatic pick, not a fill.
    pub provisional: bool,
    /// The scroll position, which the element records.
    pub scroll: Option<ScrollMemo>,
}

impl Position {
    /// The same position, with `scroll`.
    pub fn with_scroll(mut self, scroll: ScrollMemo) -> Self {
        self.scroll = Some(scroll);
        self
    }

    /// How many ids it holds, as [`RestoreMemory`]'s id budget counts them.
    fn id_count(&self) -> usize {
        self.selected.len()
    }
}

/// A scroll offset and the layout it was measured in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMemo {
    /// The layout the offset belongs to, as the element names it.
    pub layout: u64,
    /// The vertical scroll offset, in points.
    pub offset: f32,
}

impl ScrollMemo {
    /// An offset measured in `layout`.
    pub const fn new(layout: u64, offset: f32) -> Self {
        Self { layout, offset }
    }

    /// The offset, if `layout` is the one it was measured in.
    pub fn offset_for(&self, layout: u64) -> Option<f32> {
        (self.layout == layout).then_some(self.offset)
    }
}

/// Remembered positions per folder key, for the folders most recently left.
#[derive(Clone, Debug)]
pub struct RestoreMemory<K> {
    capacity: usize,
    id_budget: usize,
    ids: usize,
    clock: u64,
    folders: HashMap<K, (u64, Position)>,
}

impl<K: Eq + Hash> Default for RestoreMemory<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash> RestoreMemory<K> {
    /// How many folders it holds by default: Strata remembers the last 128 folders left.
    pub const DEFAULT_CAPACITY: usize = 128;

    /// How many selected ids it holds in all by default.
    pub const DEFAULT_ID_BUDGET: usize = 1_000_000;

    /// An empty memory with the default limits.
    pub fn new() -> Self {
        Self::with_limits(Self::DEFAULT_CAPACITY, Self::DEFAULT_ID_BUDGET)
    }

    /// An empty memory for at most `capacity` folders and `id_budget` selected ids in all.
    pub fn with_limits(capacity: usize, id_budget: usize) -> Self {
        Self {
            capacity,
            id_budget,
            ids: 0,
            clock: 0,
            folders: HashMap::new(),
        }
    }

    /// Remembers `position` for the folder `key`, as the folder left most recently.
    pub fn remember(&mut self, key: K, position: Position) -> bool {
        self.forget(&key);
        let count = position.id_count();
        if count > self.id_budget || self.capacity == 0 {
            return false;
        }
        while self.folders.len() >= self.capacity || self.ids.saturating_add(count) > self.id_budget
        {
            self.forget_oldest();
        }
        self.clock += 1;
        self.ids += count;
        self.folders.insert(key, (self.clock, position));
        true
    }

    /// The position remembered for `key`.
    pub fn recall(&self, key: &K) -> Option<&Position> {
        self.folders.get(key).map(|(_, position)| position)
    }

    /// Forgets `key`'s position, returning it.
    pub fn forget(&mut self, key: &K) -> Option<Position> {
        let (_, position) = self.folders.remove(key)?;
        self.ids -= position.id_count();
        Some(position)
    }

    /// How many folders it holds.
    pub fn len(&self) -> usize {
        self.folders.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    /// The most folders it holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The most selected ids it holds in all.
    pub fn id_budget(&self) -> usize {
        self.id_budget
    }

    /// How many selected ids it holds in all.
    pub fn ids(&self) -> usize {
        self.ids
    }

    /// Appends every `EntryId` referenced by a remembered folder position (cursor, anchor, and selected entries) into `out`.
    pub fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        for (_, position) in self.folders.values() {
            if let Some(id) = position.cursor {
                out.extend(std::iter::once(id));
            }
            if let Some(id) = position.anchor {
                out.extend(std::iter::once(id));
            }
            out.extend(position.selected.iter().copied());
        }
    }

    fn forget_oldest(&mut self) {
        let Some(oldest) = self.folders.values().map(|(left, _)| *left).min() else {
            return;
        };
        let mut freed = 0;
        self.folders.retain(|_, (left, position)| {
            let keep = *left != oldest;
            if !keep {
                freed = position.id_count();
            }
            keep
        });
        self.ids -= freed;
    }
}
