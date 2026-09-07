// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded GIO directory measurement, independent of browser widgets.

use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    rc::Rc,
    time::{Duration, Instant},
};

use gio::prelude::*;

#[derive(Default)]
pub(crate) struct DirectorySummary {
    pub(crate) item_count: usize,
    pub(crate) total_size: u64,
    /// Incomplete measurements are lower bounds, not exact totals.
    pub(crate) truncated: bool,
}

impl DirectorySummary {
    fn include(&mut self, child: Self) {
        self.item_count = self.item_count.saturating_add(child.item_count);
        self.total_size = self.total_size.saturating_add(child.total_size);
        self.truncated |= child.truncated;
    }
}

const DIRECTORY_ATTRIBUTES: &str =
    "standard::name,standard::type,standard::is-symlink,standard::size";
const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 64;
const TIME_BUDGET: Duration = Duration::from_secs(5);

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
        DIRECTORY_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        glib::Priority::DEFAULT,
    )
    .await
}

pub(crate) async fn summarize_directory(root: &gio::File) -> Result<DirectorySummary, glib::Error> {
    summarize_directory_with_budget(root, MAX_ENTRIES, MAX_DEPTH, TIME_BUDGET).await
}

async fn summarize_directory_with_budget(
    root: &gio::File,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
) -> Result<DirectorySummary, glib::Error> {
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
) -> Result<DirectorySummary, glib::Error> {
    let mut summary = DirectorySummary::default();
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
                measure_entry(
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

type MeasurementFuture = Pin<Box<dyn Future<Output = Result<DirectorySummary, glib::Error>>>>;

fn measure_entry(
    file: gio::File,
    info: gio::FileInfo,
    depth: usize,
    budget: Rc<MeasurementBudget>,
) -> MeasurementFuture {
    Box::pin(async move {
        budget.visited.set(budget.visited.get() + 1);
        let mut summary = DirectorySummary {
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

#[cfg(test)]
mod tests;
