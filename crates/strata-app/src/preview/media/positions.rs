// SPDX-License-Identifier: MIT

//! Where each video was left this session, so it reopens there (GTK's
//! `src/ui/media/positions.rs`).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::Instant,
};

/// A video left within its first second reopens at its start.
pub(super) const RESTORE_MIN_US: u64 = 1_000_000;
const MAX_REMEMBERED: usize = 128;

static POSITIONS: LazyLock<Mutex<HashMap<PathBuf, (u64, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn remember(path: PathBuf, position: u64) {
    let Ok(mut positions) = POSITIONS.lock() else {
        return;
    };
    if positions.len() >= MAX_REMEMBERED && !positions.contains_key(&path) {
        let oldest = positions
            .iter()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(path, _)| path.clone());
        if let Some(oldest) = oldest {
            positions.remove(&oldest);
        }
    }
    positions.insert(path, (position, Instant::now()));
}

pub(super) fn forget(path: &Path) {
    if let Ok(mut positions) = POSITIONS.lock() {
        positions.remove(path);
    }
}

/// Where `path` was left, if it was previewed this session.
pub(super) fn recall(path: &Path) -> Option<u64> {
    let mut positions = POSITIONS.lock().ok()?;
    let entry = positions.get_mut(path)?;
    entry.1 = Instant::now();
    Some(entry.0)
}

/// Where `path` reopens: past its first second, or nowhere.
pub(super) fn resume(path: &Path) -> Option<u64> {
    recall(path).filter(|&position| position > RESTORE_MIN_US)
}
