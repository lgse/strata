// SPDX-License-Identifier: MIT

//! Bounded GIO directory measurement, independent of browser widgets.

use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    rc::Rc,
    time::{Duration, Instant},
};

use gio::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirectorySummary {
    pub(crate) item_count: usize,
    pub(crate) total_size: u64,
    pub(crate) visible_file_count: usize,
    pub(crate) visible_folder_count: usize,
    /// Incomplete measurements are lower bounds, not exact totals.
    pub(crate) truncated: bool,
}

impl DirectorySummary {
    fn include(&mut self, child: Self) {
        self.item_count = self.item_count.saturating_add(child.item_count);
        self.total_size = self.total_size.saturating_add(child.total_size);
        self.visible_file_count = self
            .visible_file_count
            .saturating_add(child.visible_file_count);
        self.visible_folder_count = self
            .visible_folder_count
            .saturating_add(child.visible_folder_count);
        self.truncated |= child.truncated;
    }
}

const DIRECTORY_ATTRIBUTES: &str =
    "standard::name,standard::type,standard::is-symlink,standard::size,standard::is-hidden";
const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 64;
const TIME_BUDGET: Duration = Duration::from_secs(5);

struct MeasurementBudget {
    visited: Cell<usize>,
    deadline: Instant,
    max_entries: usize,
    max_depth: usize,
    total: Cell<DirectorySummary>,
    reported: Cell<DirectorySummary>,
    on_progress: Box<dyn Fn(DirectorySummary)>,
}

impl MeasurementBudget {
    fn report_progress(&self) {
        let total = self.total.get();
        if self.reported.replace(total) != total {
            (self.on_progress)(total);
        }
    }

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
    summarize_directory_with_progress(root, |_| {}).await
}

pub(crate) async fn summarize_directory_with_progress(
    root: &gio::File,
    on_progress: impl Fn(DirectorySummary) + 'static,
) -> Result<DirectorySummary, glib::Error> {
    summarize_directory_with_budget(root, MAX_ENTRIES, MAX_DEPTH, TIME_BUDGET, on_progress).await
}

async fn summarize_directory_with_budget(
    root: &gio::File,
    max_entries: usize,
    max_depth: usize,
    time_budget: Duration,
    on_progress: impl Fn(DirectorySummary) + 'static,
) -> Result<DirectorySummary, glib::Error> {
    on_progress(DirectorySummary::default());
    let enumerator = enumerate_children(root).await?;
    let budget = Rc::new(MeasurementBudget {
        visited: Cell::new(0),
        deadline: Instant::now() + time_budget,
        max_entries,
        max_depth,
        total: Cell::default(),
        reported: Cell::default(),
        on_progress: Box::new(on_progress),
    });
    measure_children(root, enumerator, 0, false, budget).await
}

async fn measure_children(
    directory: &gio::File,
    enumerator: gio::FileEnumerator,
    child_depth: usize,
    hidden_ancestor: bool,
    budget: Rc<MeasurementBudget>,
) -> Result<DirectorySummary, glib::Error> {
    let mut summary = DirectorySummary::default();
    'directory: loop {
        let children = match enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await
        {
            Ok(children) => children,
            Err(error) if summary.item_count == 0 => return Err(error),
            Err(_) => {
                // Keep bytes already reported to the UI if a later batch becomes unreadable.
                summary.truncated = true;
                break;
            }
        };
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
                    hidden_ancestor,
                    budget.clone(),
                )
                .await?,
            );
        }
        budget.report_progress();
        // Branch-local truncation (depth or an unreadable child) must not skip siblings.
        if budget.exhausted() {
            summary.truncated = true;
            break;
        }
    }
    budget.report_progress();
    Ok(summary)
}

type MeasurementFuture = Pin<Box<dyn Future<Output = Result<DirectorySummary, glib::Error>>>>;

fn measure_entry(
    file: gio::File,
    info: gio::FileInfo,
    depth: usize,
    hidden_ancestor: bool,
    budget: Rc<MeasurementBudget>,
) -> MeasurementFuture {
    Box::pin(async move {
        budget.visited.set(budget.visited.get() + 1);
        let is_directory = info.file_type() == gio::FileType::Directory;
        let is_hidden = hidden_ancestor || info.is_hidden();
        let mut summary = DirectorySummary {
            item_count: 1,
            total_size: if info.file_type() == gio::FileType::Regular {
                info.size().max(0) as u64
            } else {
                0
            },
            visible_file_count: usize::from(!is_hidden && !is_directory),
            visible_folder_count: usize::from(!is_hidden && is_directory),
            truncated: false,
        };
        let mut total = budget.total.get();
        total.include(summary);
        budget.total.set(total);
        if is_directory && !info.is_symlink() {
            if depth >= budget.max_depth || budget.exhausted() {
                summary.truncated = true;
            } else {
                let children = async {
                    let enumerator = enumerate_children(&file).await?;
                    measure_children(&file, enumerator, depth + 1, is_hidden, budget).await
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
