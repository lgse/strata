// SPDX-License-Identifier: MIT

//! `folder-views.toml` on disk: where it is, the read at start-up, and the save that merges
//! over what another Strata process wrote. [`super::folder_views`] is what the file holds.
//!
//! Nothing here schedules, and these are the store's tools, not an interface's. The one
//! caller of each is [`super::FolderViewStore`]: it holds the unsaved changes a save merges
//! in and the file's stamp, and a second caller in the process would lose the first's.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use super::{folder_views::FolderViews, lock_saves, read_state_file};
use crate::storage::{file_stamp, writer};

const FILE_NAME: &str = "folder-views.toml";
/// Another process saves in milliseconds; one stopped mid-save must not hold up exit.
const EXIT_LOCK_WAIT: Duration = Duration::from_secs(1);

pub fn folder_views_path() -> PathBuf {
    crate::storage::state_directory().join(FILE_NAME)
}

/// The saved values, and why saving is off for this session when the file existed but
/// could not be read. Reads on the calling thread.
///
/// Once in a process, at start-up, by whoever owns the store. It notes the file's stamp with
/// the settings writer, which is how a later re-read tells another process's write from
/// this one's: a second call notes the stamp again, and a change another process made
/// before it is then never adopted.
pub fn load_folder_views() -> (FolderViews, Option<io::Error>) {
    let path = folder_views_path();
    writer::note(&path, file_stamp(&path));
    let loaded = read_state_file(&path).and_then(|contents| match contents {
        Some(contents) => FolderViews::parse(&contents),
        None => Ok((FolderViews::default(), false)),
    });
    let (views, load_failure) = match loaded {
        Ok((views, repaired)) => {
            if repaired {
                tracing::warn!(path = %path.display(),
                    "folder settings file has invalid entries; keeping the valid ones");
            }
            (views, None)
        }
        Err(error) => (FolderViews::default(), Some(error)),
    };
    if let Some(error) = &load_failure {
        tracing::warn!(%error, path = %path.display(),
            "unable to load folder settings; using none without saving; fix the file and restart Strata");
    }
    (views, load_failure)
}

/// How one save went.
pub enum FolderViewsSave {
    /// Another process holds the lock, and this is not the save at exit: try again later.
    Busy,
    /// Written. With values, another process had saved since and they differ from what was
    /// asked: adopt them. Either way the edits the snapshot held are saved.
    Saved(Option<FolderViews>),
    /// Not written. The file was unreadable or from a newer version, or the write failed.
    Failed(io::Error),
}

/// An unreadable file, or one from a newer version, must not be overwritten.
///
/// For the store alone, on the settings writer's thread. Nothing but
/// [`super::FolderViewStore`] calls it: `snapshot` carries the unsaved
/// changes that are merged over the file, and two writers in one process would each save
/// without the other's.
pub fn save_folder_views(path: &Path, snapshot: FolderViews, exit: bool) -> FolderViewsSave {
    let lock = lock_saves(path, if exit { EXIT_LOCK_WAIT } else { Duration::ZERO });
    if let Err(error) = &lock {
        if error.kind() == io::ErrorKind::WouldBlock && !exit {
            return FolderViewsSave::Busy;
        }
        tracing::debug!(%error, path = %path.display(), "saving folder settings unlocked");
    }
    let saved = match read_state_file(path).and_then(|contents| {
        contents
            .map(|contents| FolderViews::parse(&contents).map(|(views, _)| views))
            .transpose()
    }) {
        Ok(saved) => saved,
        Err(error) => return FolderViewsSave::Failed(error),
    };
    let (written, changed) = match saved {
        Some(saved) => {
            let merged = snapshot.merged_over(saved);
            let changed = !merged.same_views(&snapshot);
            (merged, changed)
        }
        None => (snapshot, false),
    };
    let result = written.to_toml().and_then(|contents| {
        fs::create_dir_all(path.parent().unwrap_or(path))?;
        crate::storage::atomic_write(path, contents.as_bytes())
    });
    if result.is_ok() {
        writer::note(path, file_stamp(path));
    }
    drop(lock);
    match result {
        Ok(()) => FolderViewsSave::Saved(changed.then_some(written)),
        Err(error) => FolderViewsSave::Failed(error),
    }
}

#[cfg(test)]
pub(super) mod tests;
