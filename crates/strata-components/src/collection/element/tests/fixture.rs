//! A source for the element's tests: entries with dense ids, so `index_of` is a lookup in a
//! vector. (Review round 1 measured the perf test with `ListSource` too: its map made no
//! difference to the figures, since a frame resolves only the mounted rows.)

use crate::contract::{Entry, EntryId, EntryKind, EntrySource};

/// `n` entries; every tenth is a folder.
pub(crate) struct FastSource {
    ids: Vec<EntryId>,
    names: Vec<String>,
    /// Index by id, `usize::MAX` for an id not listed.
    index: Vec<usize>,
}

impl FastSource {
    pub(crate) fn new(n: usize) -> Self {
        let mut s = Self {
            ids: (0..n as u64).map(EntryId::new).collect(),
            names: (0..n).map(|i| format!("entry {i:06}.txt")).collect(),
            index: Vec::new(),
        };
        s.reindex();
        s
    }

    /// Puts `count` new entries at the top, as a live update does.
    pub(crate) fn insert_top(&mut self, count: usize) {
        let first = self.names.len() as u64;
        for k in 0..count as u64 {
            let id = first + k;
            self.names.push(format!("new {id}.txt"));
            self.ids.insert(0, EntryId::new(id));
        }
        self.reindex();
    }

    /// Removes the entries at `range`, as a delete does.
    pub(crate) fn remove(&mut self, range: std::ops::Range<usize>) {
        self.ids.drain(range);
        self.reindex();
    }

    /// Reverses the order, as a re-sort does.
    pub(crate) fn reverse(&mut self) {
        self.ids.reverse();
        self.reindex();
    }

    fn reindex(&mut self) {
        self.index = vec![usize::MAX; self.names.len()];
        for (i, id) in self.ids.iter().enumerate() {
            self.index[id.get() as usize] = i;
        }
    }
}

impl EntrySource for FastSource {
    fn len(&self) -> usize {
        self.ids.len()
    }

    fn id_at(&self, index: usize) -> EntryId {
        self.ids[index]
    }

    fn index_of(&self, id: EntryId) -> Option<usize> {
        self.index
            .get(id.get() as usize)
            .copied()
            .filter(|&i| i != usize::MAX)
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        let id = self.ids[index];
        let kind = if id.get().is_multiple_of(10) {
            EntryKind::Folder
        } else {
            EntryKind::File
        };
        Entry::new(id, &self.names[id.get() as usize], kind, "")
    }
}
