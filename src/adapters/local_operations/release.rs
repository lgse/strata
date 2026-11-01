// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use rustix::fs::{Mode, OFlags};

use super::fs_ops::fs;
use crate::{
    adapters::volume::{
        mount_lanes::{self, Class},
        mount_watch,
    },
    model::Location,
    services::file_activity::{self, Activity, Fence, Notice, OperationScope},
};

pub(crate) const RELEASE_BUDGET: Duration = Duration::from_secs(2);

pub(crate) struct ActiveOperation {
    activity: Activity,
    fences: RefCell<Vec<Fence>>,
    released: RefCell<Vec<PathBuf>>,
    real_folders: RefCell<HashMap<PathBuf, Option<PathBuf>>>,
}

impl ActiveOperation {
    pub(crate) fn begin(folders: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            activity: file_activity::begin(OperationScope {
                roots: Vec::new(),
                folders: folders.into_iter().collect(),
            }),
            fences: RefCell::default(),
            released: RefCell::default(),
            real_folders: RefCell::default(),
        }
    }

    pub(crate) fn include_folder(&self, folder: &Location) {
        self.activity.include_folder(folder);
    }

    pub(crate) fn include_trees(&self, trees: impl IntoIterator<Item = Location>) {
        self.activity.include_trees(trees);
    }

    pub(crate) async fn release(&self, roots: Vec<PathBuf>, cancellable: &gio::Cancellable) {
        self.release_with(roots, true, cancellable).await;
    }

    pub(crate) async fn release_lexically(
        &self,
        roots: Vec<PathBuf>,
        cancellable: &gio::Cancellable,
    ) {
        self.release_with(roots, false, cancellable).await;
    }

    async fn release_with(
        &self,
        mut roots: Vec<PathBuf>,
        resolve: bool,
        cancellable: &gio::Cancellable,
    ) {
        roots.retain(|root| root.is_absolute());
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            return;
        }
        let deadline = Instant::now() + RELEASE_BUDGET;
        let left = || deadline.saturating_duration_since(Instant::now());
        self.activity.include(OperationScope {
            roots: roots.clone(),
            folders: Vec::new(),
        });
        let lexical = self.activity.fence(roots.iter().cloned());
        file_activity::notify(&Notice::Releasing(&roots));
        lexical.stop_reads();
        let mut drained = vec![lexical.drained()];
        self.fences.borrow_mut().push(lexical);
        if resolve {
            let real = futures_lite::future::or(self.real_roots(&roots), async {
                glib::timeout_future(left()).await;
                Vec::new()
            })
            .await;
            if !real.is_empty() {
                let fence = self.activity.fence(real.iter().cloned());
                file_activity::notify(&Notice::Releasing(&real));
                fence.stop_reads();
                drained.push(fence.drained());
                self.fences.borrow_mut().push(fence);
                self.released.borrow_mut().extend(real);
            }
        }
        self.released.borrow_mut().extend(roots);
        futures_lite::future::or(
            async {
                for fence in drained {
                    fence.await;
                }
            },
            futures_lite::future::or(glib::timeout_future(left()), cancellable.future()),
        )
        .await;
    }

    async fn real_roots(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        let mut unknown: Vec<PathBuf> = {
            let known = self.real_folders.borrow();
            roots
                .iter()
                .filter_map(|root| root.parent())
                .filter(|folder| !known.contains_key(*folder))
                .map(Path::to_path_buf)
                .collect()
        };
        unknown.sort();
        unknown.dedup();
        if !unknown.is_empty() {
            let found = real_folders(unknown).await;
            self.real_folders.borrow_mut().extend(found);
        }
        let known = self.real_folders.borrow();
        let lexical: HashSet<&PathBuf> = roots.iter().collect();
        roots
            .iter()
            .filter_map(|root| {
                let real = known.get(root.parent()?)?.as_ref()?.join(root.file_name()?);
                (!lexical.contains(&real)).then_some(real)
            })
            .collect()
    }
}

impl Drop for ActiveOperation {
    fn drop(&mut self) {
        self.fences.get_mut().clear();
        self.activity.end();
        let lifted = std::mem::take(self.released.get_mut());
        if !lifted.is_empty() {
            file_activity::notify(&Notice::Lifted(&lifted));
        }
    }
}

async fn real_folders(folders: Vec<PathBuf>) -> Vec<(PathBuf, Option<PathBuf>)> {
    let mounts = mount_watch::latest();
    let mut groups: BTreeMap<Option<u64>, Vec<PathBuf>> = BTreeMap::new();
    for folder in folders {
        groups
            .entry(mounts.mount_for(&folder).map(|mount| mount.mount_id))
            .or_default()
            .push(folder);
    }
    let workers: Vec<_> = groups
        .into_values()
        .map(|folders| {
            gio::spawn_blocking(move || {
                folders
                    .into_iter()
                    .map(|folder| {
                        let real = real_folder(&folder);
                        (folder, real)
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut found = Vec::new();
    for worker in workers {
        if let Ok(answers) = worker.await {
            found.extend(answers);
        }
    }
    found
}

fn real_folder(folder: &Path) -> Option<PathBuf> {
    let admitted = mount_lanes::admit(folder, Class::Interactive).ok()?;
    let opened = fs().open_path(
        folder.as_os_str(),
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    );
    admitted.progress().answered(&opened);
    let opened = opened.ok()?;
    let real = std::fs::read_link(format!("/proc/self/fd/{}", opened.as_raw_fd())).ok()?;
    real.is_absolute().then_some(real)
}

pub(super) fn natives<'a>(locations: impl IntoIterator<Item = &'a Location>) -> Vec<PathBuf> {
    locations
        .into_iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect()
}

pub(super) fn parents<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> Vec<PathBuf> {
    let mut parents: Vec<PathBuf> = paths
        .into_iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect();
    parents.sort();
    parents.dedup();
    parents
}
