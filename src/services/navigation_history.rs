// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use gio::prelude::*;
use serde::{Deserialize, Serialize};

use super::search::{SearchItem, fold_for_search};

const HISTORY_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 1_000;
const MAX_RESULTS: usize = 100;
const MAX_TOTAL_RANK: f64 = 10_000.0;
const HOUR_SECONDS: u64 = 60 * 60;
const DAY_SECONDS: u64 = 24 * HOUR_SECONDS;
const WEEK_SECONDS: u64 = 7 * DAY_SECONDS;

thread_local! {
    static SHARED_HISTORY: RefCell<std::rc::Weak<NavigationHistory>> = const { RefCell::new(std::rc::Weak::new()) };
}

#[derive(Clone, Debug)]
struct HistoryEntry {
    path: PathBuf,
    rank: f64,
    last_accessed: u64,
}

#[derive(Deserialize, Serialize)]
struct StoredHistory {
    version: u32,
    entries: Vec<StoredEntry>,
}

#[derive(Deserialize, Serialize)]
struct StoredEntry {
    uri: String,
    rank: f64,
    last_accessed: u64,
}

pub(crate) struct NavigationHistory {
    path: PathBuf,
    entries: RefCell<Vec<HistoryEntry>>,
}

impl NavigationHistory {
    pub(crate) fn shared() -> Rc<Self> {
        SHARED_HISTORY.with(|shared| {
            if let Some(history) = shared.borrow().upgrade() {
                return history;
            }
            let history = Rc::new(Self::open(default_history_path()));
            shared.replace(Rc::downgrade(&history));
            history
        })
    }

    pub(crate) fn open(path: PathBuf) -> Self {
        let entries = match std::fs::read(&path) {
            Ok(contents) => load_entries(&contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "folder history could not be read");
                Vec::new()
            }
        };
        Self {
            path,
            entries: RefCell::new(entries),
        }
    }

    pub(crate) fn record(&self, path: &Path) {
        self.record_at(path, unix_time());
    }

    fn record_at(&self, path: &Path, now: u64) {
        if !path.is_absolute() {
            return;
        }
        let mut entries = self.entries.borrow_mut();
        if let Some(entry) = entries.iter_mut().find(|entry| entry.path == path) {
            entry.rank += 1.0;
            entry.last_accessed = now;
        } else {
            entries.push(HistoryEntry {
                path: path.to_path_buf(),
                rank: 1.0,
                last_accessed: now,
            });
        }
        age_entries(&mut entries);
        prune_entries(&mut entries, now);
        if let Err(error) = save_entries(&self.path, &entries) {
            tracing::warn!(%error, path = %self.path.display(), "folder history could not be saved");
        }
    }

    pub(crate) fn search(&self, query: &str) -> Vec<SearchItem> {
        self.search_at(query, unix_time())
    }

    /// Minimal-mode `Z`: the same visits sorted by recency (`last_accessed`
    /// descending). Empty-query `search("")` is frecency, not this.
    pub(crate) fn recent(&self) -> Vec<SearchItem> {
        let mut entries: Vec<_> = self.entries.borrow().iter().cloned().collect();
        entries.sort_unstable_by(|left, right| {
            right
                .last_accessed
                .cmp(&left.last_accessed)
                .then_with(|| left.path.cmp(&right.path))
        });
        entries
            .into_iter()
            .take(MAX_RESULTS)
            .map(|entry| SearchItem::for_history(entry.path))
            .collect()
    }

    fn search_at(&self, query: &str, now: u64) -> Vec<SearchItem> {
        let query = fold_for_search(query.trim());
        let mut matches = self
            .entries
            .borrow()
            .iter()
            .filter_map(|entry| {
                let item = SearchItem::for_history(entry.path.clone());
                let text_score = if query.is_empty() {
                    0
                } else {
                    item.fuzzy_score(&query)?
                };
                let frecency = frecency_score(entry, now);
                let frecency_bonus = (frecency.max(0.0).ln_1p() * 256.0).min(2_000.0) as i64;
                let score = if query.is_empty() {
                    (frecency * 1_000.0) as i64
                } else {
                    text_score + frecency_bonus
                };
                Some((score, entry.last_accessed, item))
            })
            .collect::<Vec<_>>();
        matches.sort_unstable_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.path.cmp(&right.2.path))
        });
        matches
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, _, item)| item)
            .collect()
    }
}

fn default_history_path() -> PathBuf {
    glib::user_state_dir()
        .join("strata")
        .join("navigation-history.json")
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn frecency_score(entry: &HistoryEntry, now: u64) -> f64 {
    let age = now.saturating_sub(entry.last_accessed);
    let multiplier = if age < HOUR_SECONDS {
        4.0
    } else if age < DAY_SECONDS {
        2.0
    } else if age < WEEK_SECONDS {
        0.5
    } else {
        0.25
    };
    entry.rank * multiplier
}

fn age_entries(entries: &mut Vec<HistoryEntry>) {
    let total_rank = entries.iter().map(|entry| entry.rank).sum::<f64>();
    if total_rank <= MAX_TOTAL_RANK {
        return;
    }
    let factor = 0.9 * MAX_TOTAL_RANK / total_rank;
    for entry in entries.iter_mut() {
        entry.rank *= factor;
    }
    entries.retain(|entry| entry.rank >= 1.0);
}

fn prune_entries(entries: &mut Vec<HistoryEntry>, now: u64) {
    if entries.len() <= MAX_ENTRIES {
        return;
    }
    entries.sort_unstable_by(|left, right| {
        frecency_score(right, now)
            .total_cmp(&frecency_score(left, now))
            .then_with(|| right.last_accessed.cmp(&left.last_accessed))
    });
    entries.truncate(MAX_ENTRIES);
}

fn load_entries(contents: &[u8]) -> Vec<HistoryEntry> {
    let Ok(stored) = serde_json::from_slice::<StoredHistory>(contents) else {
        return Vec::new();
    };
    if stored.version != HISTORY_VERSION {
        return Vec::new();
    }
    let mut entries = Vec::<HistoryEntry>::new();
    for stored in stored.entries {
        if !stored.rank.is_finite() || stored.rank <= 0.0 {
            continue;
        }
        let Some(path) = gio::File::for_uri(&stored.uri).path() else {
            continue;
        };
        if !path.is_absolute() {
            continue;
        }
        if let Some(existing) = entries.iter_mut().find(|entry| entry.path == path) {
            existing.rank += stored.rank;
            existing.last_accessed = existing.last_accessed.max(stored.last_accessed);
        } else {
            entries.push(HistoryEntry {
                path,
                rank: stored.rank,
                last_accessed: stored.last_accessed,
            });
        }
    }
    prune_entries(&mut entries, unix_time());
    entries
}

fn save_entries(path: &Path, entries: &[HistoryEntry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stored = StoredHistory {
        version: HISTORY_VERSION,
        entries: entries
            .iter()
            .map(|entry| StoredEntry {
                uri: gio::File::for_path(&entry.path).uri().to_string(),
                rank: entry.rank,
                last_accessed: entry.last_accessed,
            })
            .collect(),
    };
    let contents = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
    crate::storage::atomic_write(path, &contents)
}

#[cfg(test)]
mod tests;
