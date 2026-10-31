// SPDX-License-Identifier: MIT
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct Change {
    generation: u64,
    revision: Option<u64>,
}

#[derive(Default)]
pub(super) struct Freshness {
    generation: u64,
    global: Change,
    scopes: HashMap<String, Change>,
}

fn overlaps(a: &str, b: &str) -> bool {
    fn contains(root: &str, path: &str) -> bool {
        root == path
            || path
                .strip_prefix(root.trim_end_matches('/'))
                .is_some_and(|tail| tail.starts_with('/'))
    }
    contains(a, b) || contains(b, a)
}

impl Freshness {
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn invalidate(&mut self, paths: Option<&[String]>, revision: Option<u64>) {
        self.generation += 1;
        let change = Change {
            generation: self.generation,
            revision,
        };
        if let Some(paths) = paths {
            for path in paths {
                let entry = self.scopes.entry(path.clone()).or_default();
                entry.generation = change.generation;
                entry.revision = entry.revision.into_iter().chain(revision).max();
            }
            if self.scopes.len() <= 2048 {
                return;
            }
            self.global = Change {
                generation: change.generation,
                revision: self
                    .scopes
                    .values()
                    .filter_map(|c| c.revision)
                    .chain(self.global.revision)
                    .max(),
            };
            self.scopes.clear();
            return;
        }
        self.global = Change {
            generation: change.generation,
            revision: self.global.revision.into_iter().chain(revision).max(),
        };
        if revision.is_some() {
            self.scopes.clear();
        }
    }

    fn required(&self, paths: &[String]) -> Change {
        self.scopes
            .iter()
            .filter(|(root, _)| paths.iter().any(|path| overlaps(root, path)))
            .fold(self.global, |mut required, (_, change)| {
                required.generation = required.generation.max(change.generation);
                required.revision = required.revision.into_iter().chain(change.revision).max();
                required
            })
    }

    pub(super) fn current(&self, paths: &[String], generation: u64) -> bool {
        generation >= self.required(paths).generation
    }

    pub(super) fn accept(&self, paths: &[String], revision: Option<u64>) -> bool {
        self.required(paths)
            .revision
            .is_none_or(|required| revision.is_some_and(|at| at >= required))
    }
}
