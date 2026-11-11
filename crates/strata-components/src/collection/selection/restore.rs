//! Position restore: the cursor, selection and scroll position remembered per folder, so
//! returning to a folder (or refreshing it) puts them back.

use std::collections::HashMap;
use std::hash::Hash;

use crate::contract::EntryId;

#[cfg(test)]
mod tests;

/// One folder's remembered position: [`Selection::position`](super::Selection::position)
/// when it was left, plus the element's scroll position.
///
/// Everything but the scroll offset is by entry id, so it carries between views: a folder
/// left in Icons comes back in List with the same entries selected.
///
/// Used by: C1e (position restore), C18 (the selection before a filter), hosts.
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
///
/// An offset means different things per view, and an icons grid of another width wraps
/// differently, so the offset comes back only in the same layout. Otherwise the element
/// scrolls the cursor into view instead.
///
/// Used by: C1e, C3 (its layout key folds in the grid width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMemo {
    /// The layout the offset belongs to, as the element names it: an opaque key that folds
    /// in the view mode and anything that changes wrapping (an icons grid's width).
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
///
/// It holds at most [`capacity`](Self::capacity) folders and
/// [`id_budget`](Self::id_budget) selected ids in all. Remembering past either limit forgets
/// the folders left longest ago first. Recalling a folder doesn't count as leaving it.
///
/// Used by: C1e, hosts (one per pane or window).
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

    /// Remembers `position` for the folder `key`, as the folder left most recently, and
    /// forgets the folders left longest ago until both limits hold.
    ///
    /// Returns false, keeping nothing for `key`, when the position alone holds more ids than
    /// the budget.
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

    fn forget_oldest(&mut self) {
        // A linear scan: a memory holds about a hundred folders, and this runs once per
        // folder left. Stamps are unique, so the oldest stamp names one folder.
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
