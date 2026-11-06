// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::Instant,
};

pub(super) const RESTORE_MIN_US: u64 = 1_000_000;
const MAX_REMEMBERED_POSITIONS: usize = 128;

static MEDIA_POSITIONS: LazyLock<Mutex<HashMap<PathBuf, (u64, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn remember_media_position(path: PathBuf, position: u64) {
    if let Ok(mut positions) = MEDIA_POSITIONS.lock() {
        if positions.len() >= MAX_REMEMBERED_POSITIONS && !positions.contains_key(&path) {
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
}

pub(super) fn forget_media_position(path: &Path) {
    if let Ok(mut positions) = MEDIA_POSITIONS.lock() {
        positions.remove(path);
    }
}

pub(crate) fn recall_media_position(path: &Path) -> Option<u64> {
    MEDIA_POSITIONS.lock().ok().and_then(|mut positions| {
        let entry = positions.get_mut(path)?;
        entry.1 = Instant::now();
        Some(entry.0)
    })
}

pub(crate) fn resume_position(path: &Path) -> Option<u64> {
    recall_media_position(path).filter(|&position| position > RESTORE_MIN_US)
}
