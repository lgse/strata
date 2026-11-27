// SPDX-License-Identifier: MIT

//! "Extract here" on several archives: one extraction after another, each through the
//! single-archive pipeline, with one report when the last has ended.

use std::collections::VecDeque;

use rune::Tx;
use strata_core::{
    i18n::{count, quoted, tr},
    model::{FileEntry, Location},
};
use strata_fs::services::TransferReport;

use crate::{
    App,
    error_dialog::{self, Error},
    transfer,
};

/// Failed archives named in the closing report before the rest are counted.
const LISTED_FAILURES: usize = 8;

struct Failure {
    archive: String,
    detail: String,
}

pub(super) struct Batch {
    pending: VecDeque<FileEntry>,
    total: usize,
    started: usize,
    /// The archive extracting, or waiting on its password.
    current: String,
    extracting: bool,
    succeeded: usize,
    /// Where each archive went and the name it left to select.
    extracted: Vec<(Location, String)>,
    failures: Vec<Failure>,
    report: TransferReport,
}

impl Batch {
    fn new(entries: Vec<FileEntry>) -> Self {
        Self {
            total: entries.len(),
            pending: entries.into(),
            started: 0,
            current: String::new(),
            extracting: false,
            succeeded: 0,
            extracted: Vec::new(),
            failures: Vec::new(),
            report: TransferReport::default(),
        }
    }

    pub(super) fn position(&self) -> (usize, usize) {
        (self.started, self.total)
    }

    pub(super) fn extracting(&self) -> bool {
        self.extracting
    }

    pub(super) fn pause(&mut self) {
        self.extracting = false;
    }

    pub(super) fn resume(&mut self) {
        self.extracting = true;
    }

    pub(super) fn absorb(&mut self, report: TransferReport) {
        self.report.extend(report);
    }

    fn fail(&mut self, detail: String) {
        self.failures.push(Failure {
            archive: self.current.clone(),
            detail,
        });
    }

    /// The failed archives and what the others reported, when there is either.
    pub(super) fn detail(&self) -> Option<String> {
        let mut blocks: Vec<String> = self
            .failures
            .iter()
            .take(LISTED_FAILURES)
            .map(|failure| format!("• {}\n{}", quoted(&failure.archive), failure.detail))
            .collect();
        if self.failures.len() > LISTED_FAILURES {
            blocks.push(count("more_items", self.failures.len() - LISTED_FAILURES));
        }
        blocks.extend(transfer::reported(&self.report));
        (!blocks.is_empty()).then(|| blocks.join("\n\n"))
    }

    fn present(self, app: &mut App, tx: &mut Tx) {
        if self.failures.is_empty() {
            transfer::present_report(app, &self.report, tx);
            return;
        }
        let detail = self.detail().unwrap_or_default();
        let error = if self.succeeded == 0 {
            Error::new(tr("Unable to complete operation"), detail)
        } else {
            Error::partial_failure(detail)
        };
        error_dialog::show(app, error);
    }

    /// The names to select in the folder the last extracted archive went to.
    fn results(&self) -> Option<(Location, Vec<String>)> {
        let (folder, _) = self.extracted.last()?;
        let names = self
            .extracted
            .iter()
            .filter(|(destination, _)| destination == folder)
            .map(|(_, name)| name.clone())
            .collect();
        Some((folder.clone(), names))
    }
}

pub(super) fn start(app: &mut App, entries: Vec<FileEntry>, tx: &mut Tx) {
    app.archive.batch = Some(Batch::new(entries));
    start_next(app, tx);
}

fn start_next(app: &mut App, tx: &mut Tx) {
    loop {
        let Some(batch) = &mut app.archive.batch else {
            return;
        };
        batch.extracting = false;
        let Some(entry) = batch.pending.pop_front() else {
            return finish(app, tx);
        };
        batch.started += 1;
        batch.current.clone_from(&entry.display_name);
        let Some(parent) = entry.location.parent() else {
            batch.fail(tr("This archive has no parent directory."));
            continue;
        };
        if app.browser.can_start_operation() {
            batch.extracting = true;
        } else {
            // The browser says why below, and then no outcome would move the batch on.
            finish(app, tx);
        }
        super::begin_extract(app, entry, parent, tx);
        return;
    }
}

/// The current archive ended: `extracted` is where it went and the name to select there.
pub(super) fn on_archive_done(
    app: &mut App,
    extracted: Option<(Location, String)>,
    cancelled: bool,
    tx: &mut Tx,
) -> bool {
    let Some(batch) = &mut app.archive.batch else {
        return false;
    };
    batch.extracting = false;
    if cancelled {
        // `OperationCancelled` follows and takes what the batch has to report.
        return false;
    }
    batch.succeeded += 1;
    batch.extracted.extend(extracted);
    start_next(app, tx);
    true
}

/// Records the current archive's failure and moves on; `false` outside a batch.
pub(super) fn on_archive_failed(app: &mut App, detail: &str, tx: &mut Tx) -> bool {
    let Some(batch) = &mut app.archive.batch else {
        return false;
    };
    batch.fail(detail.to_owned());
    start_next(app, tx);
    true
}

/// Ends the batch where it stands: selects what was extracted and reports what was not.
pub(super) fn finish(app: &mut App, tx: &mut Tx) {
    let Some(batch) = app.archive.batch.take() else {
        return;
    };
    if let Some((folder, names)) = batch.results() {
        super::show_archive_result(app, Some(folder), names, tx);
    }
    batch.present(app, tx);
}
