// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded GIO Trash measurement and deletion, independent of browser widgets.

use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    rc::Rc,
    time::{Duration, Instant},
};

use gio::prelude::*;

#[derive(Default)]
pub(crate) struct TrashSummary {
    pub(crate) item_count: usize,
    pub(crate) total_size: u64,
    /// Incomplete measurements are lower bounds, not exact totals.
    pub(crate) truncated: bool,
}

impl TrashSummary {
    fn include(&mut self, child: Self) {
        self.item_count = self.item_count.saturating_add(child.item_count);
        self.total_size = self.total_size.saturating_add(child.total_size);
        self.truncated |= child.truncated;
    }
}

const TRASH_ATTRIBUTES: &str = "standard::display-name,standard::name,standard::type,standard::is-symlink,standard::size,time::modified";
const MAX_TRASH_ENTRIES: usize = 200_000;
const MAX_TRASH_DEPTH: usize = 64;
const TRASH_TIME_BUDGET: Duration = Duration::from_secs(5);

struct MeasurementBudget {
    visited: Cell<usize>,
    deadline: Instant,
    max_entries: usize,
    max_depth: usize,
}

impl MeasurementBudget {
    fn exhausted(&self) -> bool {
        self.visited.get() >= self.max_entries || Instant::now() >= self.deadline
    }
}

async fn enumerate_children(file: &gio::File) -> Result<gio::FileEnumerator, glib::Error> {
    file.enumerate_children_future(
        TRASH_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        glib::Priority::DEFAULT,
    )
    .await
}

pub(crate) async fn summarize_trash(root: &gio::File) -> Result<TrashSummary, glib::Error> {
    summarize_trash_with_budget(root, MAX_TRASH_ENTRIES, MAX_TRASH_DEPTH, TRASH_TIME_BUDGET).await
}

async fn summarize_trash_with_budget(
    root: &gio::File,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> Result<TrashSummary, glib::Error> {
    let enumerator = enumerate_children(root).await?;
    let budget = Rc::new(MeasurementBudget {
        visited: Cell::new(0),
        deadline: Instant::now() + time_budget,
        max_entries,
        max_depth,
    });
    measure_children(root, enumerator, 0, budget).await
}

async fn measure_children(
    directory: &gio::File,
    enumerator: gio::FileEnumerator,
    child_depth: usize,
    budget: Rc<MeasurementBudget>,
) -> Result<TrashSummary, glib::Error> {
    let mut summary = TrashSummary::default();
    'directory: loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        glib::timeout_future(Duration::from_millis(1)).await;
        for info in children {
            if budget.exhausted() {
                summary.truncated = true;
                break 'directory;
            }
            summary.include(
                measure_trash_entry(
                    directory.child(info.name()),
                    info,
                    child_depth,
                    budget.clone(),
                )
                .await?,
            );
        }
        // Branch-local truncation (depth or an unreadable child) must not skip siblings.
        if budget.exhausted() {
            summary.truncated = true;
            break;
        }
    }
    Ok(summary)
}

type TrashMeasurementFuture = Pin<Box<dyn Future<Output = Result<TrashSummary, glib::Error>>>>;

fn measure_trash_entry(
    file: gio::File,
    info: gio::FileInfo,
    depth: usize,
    budget: Rc<MeasurementBudget>,
) -> TrashMeasurementFuture {
    Box::pin(async move {
        budget.visited.set(budget.visited.get() + 1);
        let mut summary = TrashSummary {
            item_count: 1,
            total_size: if info.file_type() == gio::FileType::Regular {
                info.size().max(0) as u64
            } else {
                0
            },
            truncated: false,
        };
        if info.file_type() == gio::FileType::Directory && !info.is_symlink() {
            if depth >= budget.max_depth || budget.exhausted() {
                summary.truncated = true;
            } else {
                let children = async {
                    let enumerator = enumerate_children(&file).await?;
                    measure_children(&file, enumerator, depth + 1, budget).await
                }
                .await;
                match children {
                    Ok(children) => summary.include(children),
                    // Disappearing or unreadable children do not invalidate unrelated branches.
                    Err(_) => summary.truncated = true,
                }
            }
        }
        Ok(summary)
    })
}

pub(crate) struct EmptyTrashOutcome {
    pub(crate) deleted: usize,
    pub(crate) failed: usize,
    /// Capped at 8 messages regardless of `failed`.
    pub(crate) errors: Vec<String>,
}

/// Deletes one batch at a time, independently of any prior measurement's budget or truncation.
pub(crate) async fn empty_trash(
    root: &gio::File,
    mut on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    let enumerator = enumerate_children(root).await?;
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

#[cfg(test)]
mod tests;
