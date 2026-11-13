// SPDX-License-Identifier: MIT

//! The per-folder views a process holds, and the bookkeeping of saving them: one for the
//! process, held by the interface however it holds state. [`super::folder_views`] is the
//! model and [`super::folder_view_file`] the file; this is what stands between them.
//!
//! Nothing here schedules or calls back. The interface owns the clock and whatever follows
//! the file: it changes the values, decides when to write, awaits the write where it awaits
//! things, and tells the store how it went. What every interface keeps to, because the
//! GTK application does:
//!
//! - **One store for the file in a process.** [`FolderViewStore::load`] notes the file's
//!   stamp with the settings writer, and a write merges this store's unsaved changes over
//!   the file: a second store would save without the first's.
//! - **A write is asked for [`SAVE_DELAY`] after a change and [`USE_SAVE_DELAY`] after a
//!   folder was only opened,** on one timer: a pending write keeps its time unless a
//!   shorter wait is asked for.
//! - **[`Written::Busy`] is tried again** [`SAVE_DELAY`] later.
//! - **A write that reached the file is settled in three steps,** in this order: the
//!   interface notes the save, adopts what another process had saved and shows it
//!   ([`FolderViewStore::adopt`]), and then counts the write's own changes as saved
//!   ([`FolderViewStore::finish_write`]).
//! - **At exit** a store with unsaved changes writes once more, with `exit` set, which
//!   waits briefly for another process's save and then goes ahead.
//! - **When the file changes under the process,** [`FolderViewStore::reread`] and then
//!   [`FolderViewStore::adopt`].

use std::{future::Future, io, path::PathBuf, pin::Pin, rc::Rc, time::Duration};

use super::{
    folder_view_file::{FolderViewsSave, folder_views_path, load_folder_views, save_folder_views},
    folder_views::FolderViews,
};
use crate::storage::writer;

/// Coalesces bursts of changes, such as a thumbnail-size drag, into one write
/// that stays off the click that made them.
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
    /// Written. With `merged`, another process had saved since and its values differ from
    /// what was asked: [`FolderViewStore::adopt`] them, and show what changed. Then, with
    /// or without, [`FolderViewStore::finish_write`].
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

/// Why no write was started: the file could not be read at start-up, so saving stays off
/// to keep it. It borrows nothing from the store, so an interface can show it with the
/// store free.
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

    /// The values, to change. The model counts each change, so the store needs no telling:
    /// the interface asks for a write when the change says something changed.
    pub fn views_mut(&mut self) -> &mut FolderViews {
        &mut self.views
    }

    /// Why saving is off for this session. With one, a write is not delayed: it is asked for
    /// at once, and [`Self::begin_write`] answers with the reason for the interface to show.
    pub fn load_failure(&self) -> Option<&io::Error> {
        self.load_failure.as_deref()
    }

    /// Starts a write of the values as they are now, on the settings writer's thread.
    /// `exit` is the write before the process ends, which does not come back
    /// [`Written::Busy`].
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

    /// Counts the changes a write carried as saved. Changes made since stay unsaved.
    /// After [`Self::adopt`], when the write came back with another process's values: the
    /// GTK application shows an adoption before it settles the write, and so does this.
    pub fn finish_write(&mut self, write: SavedWrite) {
        self.views.mark_saved_through(write.edits);
    }

    /// Reads the file again, on the settings writer's thread, when it changed under the
    /// process. `None` when this process wrote it last, when it is gone, or when it cannot be
    /// read: there is then nothing to adopt.
    pub fn reread() -> impl Future<Output = Option<FolderViews>> {
        let saved = writer::reread(folder_views_path(), |contents| {
            FolderViews::parse(contents).map(|(views, _)| views)
        });
        async move { saved.await.ok().and_then(Result::ok).flatten() }
    }

    /// Merges this store's unsaved changes over what another process saved, and says
    /// whether the values changed.
    pub fn adopt(&mut self, saved: FolderViews) -> bool {
        let merged = self.views.merged_over(saved);
        let changed = !merged.same_views(&self.views);
        self.views = merged;
        changed
    }
}

#[cfg(test)]
mod tests;
