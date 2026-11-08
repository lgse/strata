// SPDX-License-Identifier: MIT

//! Bounded GIO Trash deletion, independent of browser widgets.

use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::{ActiveOperation, MountTable, mount_watch};

pub(crate) struct EmptyTrashOutcome {
    pub(crate) deleted: usize,
    pub(crate) failed: usize,
    /// Capped at 8 messages regardless of `failed`.
    pub(crate) errors: Vec<String>,
}

impl EmptyTrashOutcome {
    async fn record_deletion(
        &mut self,
        file: &gio::File,
        info: &gio::FileInfo,
        result: Result<(), glib::Error>,
    ) {
        match result {
            Ok(()) => self.deleted += 1,
            Err(error) => {
                let absent = error.matches(gio::IOErrorEnum::NotFound)
                    || (!error.matches(gio::IOErrorEnum::Cancelled)
                        && file
                            .query_info_future(
                                "standard::type",
                                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                                glib::Priority::DEFAULT,
                            )
                            .await
                            .is_err_and(|error| error.matches(gio::IOErrorEnum::NotFound)));
                if absent {
                    self.deleted += 1;
                    return;
                }
                self.failed += 1;
                if self.errors.len() < 8 {
                    self.errors
                        .push(format!("{}: {error}", info.display_name()));
                }
            }
        }
    }
}

/// Deletes one batch at a time, independently of any prior measurement's budget or truncation.
pub(crate) async fn empty_trash(
    root: &gio::File,
    on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    let roots = trash_file_roots(
        &glib::user_data_dir().join("Trash"),
        &mount_watch::latest(),
        rustix::process::getuid().as_raw(),
    );
    empty_releasing(root, roots, on_progress).await
}

fn trash_file_roots(home_trash: &Path, mounts: &MountTable, uid: u32) -> Vec<PathBuf> {
    let mut roots = vec![home_trash.join("files")];
    for mount in mounts.non_virtual_mounts() {
        roots.push(mount.join(format!(".Trash-{uid}")).join("files"));
        roots.push(mount.join(".Trash").join(uid.to_string()).join("files"));
    }
    roots
}

async fn empty_releasing(
    root: &gio::File,
    roots: Vec<PathBuf>,
    mut on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    let operation = ActiveOperation::begin([]);
    operation
        .release_lexically(roots, &gio::Cancellable::new())
        .await;
    let enumerator = root
        .enumerate_children_future(
            "standard::name,standard::display-name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut outcome = EmptyTrashOutcome {
        deleted: 0,
        failed: 0,
        errors: Vec::new(),
    };
    loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        for info in children {
            let file = root.child(info.name());
            let result = file.delete_future(glib::Priority::DEFAULT).await;
            outcome.record_deletion(&file, &info, result).await;
        }
        on_progress(outcome.deleted + outcome.failed);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
