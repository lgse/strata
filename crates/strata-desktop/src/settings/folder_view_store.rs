// SPDX-License-Identifier: MIT

use std::{future::Future, io, path::PathBuf, pin::Pin, rc::Rc, time::Duration};

use super::{
    folder_view_file::{FolderViewsSave, folder_views_path, load_folder_views, save_folder_views},
    folder_views::FolderViews,
};
use crate::storage::writer;

pub const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Opening folders only refreshes use times, which can wait for other changes.
pub const USE_SAVE_DELAY: Duration = Duration::from_secs(60);

/// The values in memory and why saving is off, if it is.
pub struct FolderViewStore {
    views: FolderViews,
    /// Why saving is off for this session: the file existed but could not be read.
    load_failure: Option<Rc<io::Error>>,
}

/// A write on its way to the file.
pub struct PendingWrite {
    path: PathBuf,
    edits: u64,
    done: Pin<Box<dyn Future<Output = FolderViewsSave>>>,
}

impl PendingWrite {
    /// Waits for the writer. A writer that went away counts as a cancelled write.
    pub async fn finished(self) -> Written {
        match self.done.await {
            FolderViewsSave::Busy => Written::Busy,
            FolderViewsSave::Failed(error) => Written::Failed {
                path: self.path,
                error,
            },
            FolderViewsSave::Saved(merged) => Written::Saved {
                merged,
                write: SavedWrite { edits: self.edits },
            },
        }
    }
}

/// How a write went, for the interface to act on.
pub enum Written {
    /// Another process was saving: ask for a write again, [`SAVE_DELAY`] from now.
    Busy,
    /// Not written. The changes stay unsaved, and the next write tries again.
    Failed { path: PathBuf, error: io::Error },
    Saved {
        merged: Option<FolderViews>,
        write: SavedWrite,
    },
}

/// A write that reached the file, until [`FolderViewStore::finish_write`] takes it.
#[must_use = "the changes a write carried stay unsaved until finish_write takes it"]
pub struct SavedWrite {
    edits: u64,
}

pub struct UnreadableAtStartup {
    pub path: PathBuf,
    pub error: Rc<io::Error>,
}

impl FolderViewStore {
    /// Reads the file, on the calling thread. Once in a process, at start-up.
    pub fn load() -> Self {
        let (views, load_failure) = load_folder_views();
        Self {
            views,
            load_failure: load_failure.map(Rc::new),
        }
    }

    pub fn views(&self) -> &FolderViews {
        &self.views
    }

    pub fn views_mut(&mut self) -> &mut FolderViews {
        &mut self.views
    }

    pub fn load_failure(&self) -> Option<&io::Error> {
        self.load_failure.as_deref()
    }

    pub fn begin_write(&self, exit: bool) -> Result<PendingWrite, UnreadableAtStartup> {
        let path = folder_views_path();
        if let Some(error) = &self.load_failure {
            return Err(UnreadableAtStartup {
                path,
                error: error.clone(),
            });
        }
        let snapshot = self.views.clone();
        let edits = snapshot.edits();
        let saving = path.clone();
        let saved = writer::run(move || save_folder_views(&saving, snapshot, exit));
        Ok(PendingWrite {
            path,
            edits,
            done: Box::pin(async move {
                saved.await.unwrap_or_else(|_| {
                    FolderViewsSave::Failed(io::Error::from(rustix::io::Errno::CANCELED))
                })
            }),
        })
    }

    pub fn finish_write(&mut self, write: SavedWrite) {
        self.views.mark_saved_through(write.edits);
    }

    pub fn reread() -> impl Future<Output = Option<FolderViews>> {
        let saved = writer::reread(folder_views_path(), |contents| {
            FolderViews::parse(contents).map(|(views, _)| views)
        });
        async move { saved.await.ok().and_then(Result::ok).flatten() }
    }

    pub fn adopt(&mut self, saved: FolderViews) -> bool {
        let merged = self.views.merged_over(saved);
        let changed = !merged.same_views(&self.views);
        self.views = merged;
        changed
    }
}

#[cfg(test)]
mod tests;
