// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(crate) struct TrashSummary {
    pub(crate) item_count: usize,
    pub(crate) total_size: u64,
    /// `true` if measurement did not cover the full trash tree; `item_count`/`total_size` are
    /// then a lower bound.
    pub(crate) truncated: bool,
}

const TRASH_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-symlink,standard::size,time::modified";

const MAX_TRASH_ENTRIES: usize = 200_000;

const MAX_TRASH_DEPTH: usize = 64;

const TRASH_TIME_BUDGET: Duration = Duration::from_secs(5);

pub(crate) async fn summarize_trash(root: &gio::File) -> Result<TrashSummary, glib::Error> {
    summarize_trash_with_budget(root, MAX_TRASH_ENTRIES, MAX_TRASH_DEPTH, TRASH_TIME_BUDGET).await
}

async fn summarize_trash_with_budget(
    root: &gio::File,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> Result<TrashSummary, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut item_count = 0_usize;
    let mut total_size = 0_u64;
    let mut truncated = false;
    let visited = Rc::new(Cell::new(0_usize));
    let deadline = Instant::now() + time_budget;
    'root: loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        glib::timeout_future(Duration::from_millis(1)).await;
        for info in children {
            if visited.get() >= max_entries || Instant::now() >= deadline {
                truncated = true;
                break 'root;
            }
            let (count, size, entry_truncated) = measure_trash_entry(
                root.child(info.name()),
                info,
                0,
                visited.clone(),
                deadline,
                max_entries,
                max_depth,
            )
            .await?;
            item_count = item_count.saturating_add(count);
            total_size = total_size.saturating_add(size);
            truncated |= entry_truncated;
        }
        // Stop only when the shared budget is actually spent -- a child's own `truncated` (depth
        // cap, discarded error) is branch-local and must not cut off its unrelated siblings.
        if visited.get() >= max_entries || Instant::now() >= deadline {
            truncated = true;
            break;
        }
    }
    Ok(TrashSummary {
        item_count,
        total_size,
        truncated,
    })
}

pub(crate) struct EmptyTrashOutcome {
    pub(crate) deleted: usize,
    pub(crate) failed: usize,
    /// Capped at 8 messages regardless of `failed`, so a trash full of failures can't grow this
    /// without bound.
    pub(crate) errors: Vec<String>,
}

/// Empties `root` by enumerating and deleting one batch at a time -- unlike a listing that
/// collects every top-level entry into a `Vec<FileEntry>` first, no per-entry list is ever
/// retained here; only a running count and a capped error list, so memory stays flat no matter
/// how large the trash is.
pub(crate) async fn empty_trash(
    root: &gio::File,
    mut on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
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
            match file.delete_future(glib::Priority::DEFAULT).await {
                Ok(_) => outcome.deleted += 1,
                Err(error) => {
                    outcome.failed += 1;
                    if outcome.errors.len() < 8 {
                        outcome
                            .errors
                            .push(format!("{}: {error}", info.display_name()));
                    }
                }
            }
        }
        on_progress(outcome.deleted + outcome.failed);
    }
    Ok(outcome)
}

type TrashMeasurementFuture =
    Pin<Box<dyn Future<Output = Result<(usize, u64, bool), glib::Error>>>>;

/// `visited` is shared across the whole walk, so the entry budget applies tree-wide rather than
/// per-branch, and `deadline` is a fixed point so descending deeper can't reset the time budget.
fn measure_trash_entry(
    file: gio::File,
    info: gio::FileInfo,
    depth: usize,
    visited: Rc<Cell<usize>>,
    deadline: Instant,
    max_entries: usize,
    max_depth: usize,
) -> TrashMeasurementFuture {
    Box::pin(async move {
        visited.set(visited.get() + 1);
        let mut count = 1_usize;
        let mut size = if info.file_type() == gio::FileType::Regular {
            info.size().max(0) as u64
        } else {
            0
        };
        let mut truncated = false;
        if info.file_type() == gio::FileType::Directory && !info.is_symlink() {
            let budget_exhausted =
                depth >= max_depth || visited.get() >= max_entries || Instant::now() >= deadline;
            if budget_exhausted {
                truncated = true;
            } else {
                // A directory can become unreadable or disappear before we measure it; degrade
                // this branch to truncated rather than failing the whole walk.
                match enumerate_trash_directory(
                    &file,
                    depth,
                    visited.clone(),
                    deadline,
                    max_entries,
                    max_depth,
                )
                .await
                {
                    Ok((child_count, child_size, child_truncated)) => {
                        count = count.saturating_add(child_count);
                        size = size.saturating_add(child_size);
                        truncated |= child_truncated;
                    }
                    Err(_) => truncated = true,
                }
            }
        }
        Ok((count, size, truncated))
    })
}

async fn enumerate_trash_directory(
    file: &gio::File,
    depth: usize,
    visited: Rc<Cell<usize>>,
    deadline: Instant,
    max_entries: usize,
    max_depth: usize,
) -> Result<(usize, u64, bool), glib::Error> {
    let enumerator = file
        .enumerate_children_future(
            TRASH_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut count = 0_usize;
    let mut size = 0_u64;
    let mut truncated = false;
    'this_directory: loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        glib::timeout_future(Duration::from_millis(1)).await;
        for child in children {
            if visited.get() >= max_entries || Instant::now() >= deadline {
                truncated = true;
                break 'this_directory;
            }
            let (child_count, child_size, child_truncated) = measure_trash_entry(
                file.child(child.name()),
                child,
                depth + 1,
                visited.clone(),
                deadline,
                max_entries,
                max_depth,
            )
            .await?;
            count = count.saturating_add(child_count);
            size = size.saturating_add(child_size);
            truncated |= child_truncated;
        }
        // Stop only when the shared budget is actually spent -- a child's own `truncated` (depth
        // cap, discarded error) is branch-local and must not cut off its unrelated siblings.
        if visited.get() >= max_entries || Instant::now() >= deadline {
            truncated = true;
            break;
        }
    }
    Ok((count, size, truncated))
}

#[cfg(test)]
mod tests;
