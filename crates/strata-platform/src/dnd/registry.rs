// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use rune::Live;

use super::api::DropTarget;

pub(crate) const CAP: usize = 16_384;

pub(crate) struct Entry<S> {
    pub(crate) target: DropTarget<S>,
    /// Where the element was last drawn, `[x, y, w, h]` in window points.
    pub(crate) rect: Live<[f32; 4]>,
    stamp: u64,
}

pub(crate) struct Registry<S> {
    entries: HashMap<String, Entry<S>>,
    next: u64,
}

impl<S> Default for Registry<S> {
    fn default() -> Self {
        Registry {
            entries: HashMap::new(),
            next: 0,
        }
    }
}

impl<S: 'static> Registry<S> {
    /// Registers `target` for `key`; the rect the element reports into.
    pub(crate) fn put(&mut self, key: &str, target: DropTarget<S>) -> Live<[f32; 4]> {
        self.next += 1;
        let stamp = self.next;
        if let Some(e) = self.entries.get_mut(key) {
            e.target = target;
            e.stamp = stamp;
            return e.rect.clone();
        }
        let rect = Live::new([0.0; 4]);
        self.entries.insert(
            key.to_owned(),
            Entry {
                target,
                rect: rect.clone(),
                stamp,
            },
        );
        if self.entries.len() > CAP {
            self.evict();
        }
        rect
    }

    fn evict(&mut self) {
        let mut stamps: Vec<u64> = self.entries.values().map(|e| e.stamp).collect();
        stamps.sort_unstable();
        // Drop the oldest eighth at once, so a full registry doesn't sort on every put.
        let cut = stamps[stamps.len() / 8];
        self.entries.retain(|_, e| e.stamp > cut);
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Entry<S>> {
        self.entries.get(key)
    }

    pub(crate) fn along(&self, path: &str) -> Vec<String> {
        let segs: Vec<&str> = keyed_segments(path).collect();
        let mut out = Vec::new();
        for (i, seg) in segs.iter().enumerate().rev() {
            if self.entries.contains_key(*seg) {
                out.push((*seg).to_owned());
                continue;
            }
            if let Some(id) = segs[..i]
                .iter()
                .rev()
                .map(|scope| scoped(scope, seg))
                .find(|id| self.entries.contains_key(id))
            {
                out.push(id);
            }
        }
        out
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(crate) fn scoped(scope: &str, key: &str) -> String {
    format!("{scope}\u{1f}{key}")
}

/// A path's keyed segments, outermost first: `#n` stands for an element without a key.
fn keyed_segments(path: &str) -> impl DoubleEndedIterator<Item = &str> {
    path.split('/')
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
}

#[cfg(test)]
mod tests;
