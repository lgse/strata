// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) enum Destination {
    New(PathBuf, Taken),
    Merge(PathBuf, Option<Arc<Placing>>),
}

pub(in crate::adapters::local_operations) struct Placing {
    pub(super) root: PathBuf,
    pub(super) wanted: HashSet<PathBuf>,
    pub(super) unwanted: Option<HashSet<PathBuf>>,
    pub(super) aside: HashSet<PathBuf>,
    pub(super) placed: Mutex<Vec<(PathBuf, Placed)>>,
    pub(super) over_aside: Mutex<HashMap<PathBuf, Ownership>>,
    pub(super) left: Arc<AtomicUsize>,
}

impl Placing {
    /// Beneath `root`, `wanted` are the paths undo reverts; where the merge records all it
    /// creates, `unwanted` are the overwritten paths undo leaves.
    pub(in crate::adapters::local_operations) fn new(
        root: PathBuf,
        wanted: HashSet<PathBuf>,
        unwanted: Option<HashSet<PathBuf>>,
        aside: HashSet<PathBuf>,
        left: Arc<AtomicUsize>,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            wanted,
            unwanted,
            aside,
            placed: Mutex::default(),
            over_aside: Mutex::default(),
            left,
        })
    }

    pub(super) fn wants(&self, path: &Path) -> bool {
        self.wanted.contains(path)
    }

    pub(super) fn placed(
        &self,
        fs: &dyn FsOps,
        (dir, name): (BorrowedFd<'_>, &OsStr),
        path: &Path,
        ownership: &Ownership,
        cancelled: &AtomicBool,
    ) {
        let path = self.root.join(path);
        if self.aside.contains(&path) {
            lock(&self.over_aside).insert(path.clone(), ownership.clone());
        }
        let wanted = match &self.unwanted {
            Some(unwanted) => !unwanted.contains(&path),
            None => self.wants(&path),
        };
        if wanted {
            self.record_proven(fs, (dir, name), path, ownership, cancelled);
        }
    }

    pub(super) fn record_proven(
        &self,
        fs: &dyn FsOps,
        at: (BorrowedFd<'_>, &OsStr),
        path: PathBuf,
        ownership: &Ownership,
        cancelled: &AtomicBool,
    ) {
        match manifest::placed_proven(fs, at, cancelled, &self.left, ownership) {
            Ok(placed) => lock(&self.placed).push((path, placed)),
            Err(failure) => {
                tracing::warn!(?path, error = %failure.into_error(at.1), "could not record a placed item")
            }
        }
    }

    pub(in crate::adapters::local_operations) fn over_aside(&self) -> HashMap<PathBuf, Ownership> {
        std::mem::take(&mut *lock(&self.over_aside))
    }

    pub(in crate::adapters::local_operations) fn take(&self) -> Vec<(PathBuf, PlacedSnapshot)> {
        std::mem::take(&mut *lock(&self.placed))
            .into_iter()
            .map(|(path, placed)| (path, PlacedSnapshot::new(placed)))
            .collect()
    }
}

pub(in crate::adapters::local_operations) fn placed_budget() -> Arc<AtomicUsize> {
    Arc::new(manifest::placed_entries())
}

#[cfg(test)]
pub(in crate::adapters::local_operations) async fn placed(
    path: PathBuf,
    left: Arc<AtomicUsize>,
    cancellable: &gio::Cancellable,
) -> Option<PlacedSnapshot> {
    let recorded = run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = split(&path).ok()?;
        placed_within(fs(), (parent.as_fd(), &name), &cancelled, &left)
    })
    .await;
    recorded.ok().flatten()
}

#[cfg(test)]
pub(super) fn placed_within(
    fs: &dyn FsOps,
    (dir, name): (BorrowedFd<'_>, &OsStr),
    cancelled: &AtomicBool,
    left: &AtomicUsize,
) -> Option<PlacedSnapshot> {
    match manifest::placed(fs, (dir, name), cancelled, left) {
        Ok(placed) => Some(PlacedSnapshot::new(placed)),
        Err(failure) => {
            let error = failure.into_error(name);
            tracing::warn!(?name, %error, "could not record a placed item");
            None
        }
    }
}

pub(in crate::adapters::local_operations) async fn remove_placed(
    path: PathBuf,
    placed: PlacedSnapshot,
    unlinks: Arc<Unlinks>,
    cancellable: &gio::Cancellable,
) -> Removed {
    let failed = |error| Removed {
        kept: Vec::new(),
        removed: false,
        result: Err(error),
    };
    let removed = run_cancellable(cancellable, move |cancelled| {
        let Some(placed) = placed.get::<Placed>() else {
            return failed(translated_io_error(
                "The record of the placed item is unreadable",
            ));
        };
        let Some(folder) = path.parent() else {
            return failed(io_error(crate::i18n::tr("Invalid local filesystem target")));
        };
        let parent = match open_local_parent_directory(folder) {
            Ok(parent) => Arc::new(parent),
            Err(_)
                if std::fs::symlink_metadata(folder)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Removed {
                    kept: Vec::new(),
                    removed: false,
                    result: Ok(()),
                };
            }
            Err(error) => return failed(io_error(error)),
        };
        // Deleting only part of it would not free the path for the original.
        let compared = manifest::compare(
            fs(),
            parent.clone(),
            &placed.root(),
            unlinks.clone(),
            cancelled.clone(),
        );
        if compared.result.is_err() || !compared.kept.is_empty() {
            return compared;
        }
        manifest::remove_placed(fs(), parent, &placed.root(), unlinks, cancelled)
    })
    .await;
    removed.unwrap_or_else(|_| failed(translated_io_error("The deletion stopped unexpectedly")))
}
