// SPDX-License-Identifier: MIT

//! Hosting C16's [`DockView`] and [`ProgressDialog`] for transfer operations, and reporting
//! transfer outcomes (`src/ui/browser/progress.rs`, `events/operations.rs`, `transfer_report.rs`).

use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
};

use rune::{El, Tx};
use strata_components::progress_dock::{
    Dock, DockIntent, DockView, Notice, OpId, OpSpec, Placement, Progress, ProgressDialog, Sample,
    Transfer as TransferCounts, reveal_after,
};
use strata_core::{
    i18n::{capitalize_first, count, list, tr},
    model::Location,
};
use strata_fs::{
    app::{Browser, BrowserEvent},
    services::{
        Holder, KeptCause, KeptItem, OperationRequestId, RenameCause, RenamedItem, SkippedItem,
        SpecialKind, TransferReport, io_error_detail,
    },
};
use strata_kit::icons::IconName;

use super::words;
use crate::{
    App, clipboard,
    error_dialog::{self, Error},
    shell,
    slots::Frame,
};

const NOTICE_SECONDS: f64 = 6.0;
const NOTICE_CHARS: usize = 120;
const LISTED_ITEMS: usize = 8;

#[derive(Clone, Copy)]
struct TrackedOp {
    op_id: OpId,
    request_id: Option<OperationRequestId>,
    total_items: usize,
    flushing: bool,
    last_transfer: TransferCounts,
}

pub(super) struct Ops {
    pub(super) dock: Dock,
    next_id: OpId,
    foreground: Option<TrackedOp>,
    background: HashMap<OperationRequestId, TrackedOp>,
    pub(super) last_clock: f64,
    pub(super) queued_cancels: Vec<OpId>,
}

impl Default for Ops {
    fn default() -> Self {
        let mut dock = Dock::new();
        dock.set_words(words::translated());
        Self {
            dock,
            next_id: 0,
            foreground: None,
            background: HashMap::new(),
            last_clock: 0.0,
            queued_cancels: Vec::new(),
        }
    }
}

impl Ops {
    fn allocate_id(&mut self) -> OpId {
        self.next_id += 1;
        self.next_id
    }

    pub(super) fn foreground_op_id(&self) -> Option<OpId> {
        self.foreground.map(|op| op.op_id)
    }

    pub(super) fn dialog_holds_keyboard(&self) -> bool {
        let Some(op_id) = self.foreground_op_id() else {
            return false;
        };
        self.dock.op(op_id).is_some_and(|op| {
            op.revealed() && op.spec.placement == Placement::Dialog && op.ended().is_none()
        })
    }

    fn request_for_op(&self, op_id: OpId) -> Option<OperationRequestId> {
        if let Some(fg) = self.foreground
            && fg.op_id == op_id
        {
            return fg.request_id;
        }
        self.background
            .iter()
            .find_map(|(req_id, tracked)| (tracked.op_id == op_id).then_some(*req_id))
    }

    fn tracked_mut(&mut self, bg: Option<OperationRequestId>) -> Option<&mut TrackedOp> {
        match bg {
            Some(req_id) => self.background.get_mut(&req_id),
            None => self.foreground.as_mut(),
        }
    }

    fn take_tracked(&mut self, bg: Option<OperationRequestId>) -> Option<TrackedOp> {
        match bg {
            Some(req_id) => self.background.remove(&req_id),
            None => self.foreground.take(),
        }
    }
}

#[derive(Clone)]
pub(super) struct StartedTransfer {
    total: usize,
    moving: bool,
    info: String,
    destination: String,
    destination_description: String,
    request_id: Option<OperationRequestId>,
    backgroundable: Option<OperationRequestId>,
}

impl StartedTransfer {
    pub(super) fn capture(browser: &Browser, total: usize, moving: bool) -> Self {
        Self {
            total,
            moving,
            info: browser.operation_description(),
            destination: browser.operation_destination_label(),
            destination_description: browser.operation_destination_description(),
            request_id: browser.last_started_operation(),
            backgroundable: browser.backgroundable_operation(),
        }
    }
}

#[derive(Clone)]
pub(super) struct ProgressUpdate {
    pub(super) completed_items: usize,
    pub(super) completed_files: usize,
    pub(super) total_files: Option<usize>,
    pub(super) current_file: Option<String>,
    pub(super) transferred_bytes: u64,
    pub(super) total_bytes: Option<u64>,
}

#[derive(Clone)]
pub(super) struct CancelledOutcome {
    pub(super) completed: usize,
    pub(super) failed: usize,
    pub(super) not_attempted: usize,
    pub(super) affected_locations: HashSet<Location>,
    pub(super) report: TransferReport,
}

pub(super) fn build_overlay(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    DockView::new(
        "dock",
        |app: &App| &app.transfer.ops.dock,
        |app: &mut App| &mut app.transfer.ops.dock,
    )
    .on_intent(on_dock_intent)
    .build(app, &app.look, frame.layer);
    None
}

pub(super) fn build_dialog(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let op_id = app.transfer.ops.foreground_op_id()?;
    let mut dialog = ProgressDialog::new(
        "progress",
        |app: &App| &app.transfer.ops.dock,
        |app: &mut App| &mut app.transfer.ops.dock,
        op_id,
    )
    .reduced_motion(!app.look.animations)
    .on_intent(on_dialog_intent);
    if let Some(focus) = shell::view_focus(app) {
        dialog = dialog.return_focus(focus);
    }
    dialog.build(app, &app.look, frame.layer);
    None
}

fn on_dock_intent(app: &mut App, intent: DockIntent, _: &mut Tx) {
    if let DockIntent::Cancel(op_id) = intent {
        cancel_op(app, op_id);
    }
}

fn on_dialog_intent(app: &mut App, intent: DockIntent) {
    if let DockIntent::Cancel(op_id) = intent {
        app.transfer.ops.queued_cancels.push(op_id);
    }
}

pub(super) fn cancel_op(app: &mut App, op_id: OpId) {
    match app.transfer.ops.request_for_op(op_id) {
        Some(request_id) => app.browser.cancel_operation(request_id),
        None => app.browser.cancel_file_operation(),
    }
}

pub(super) fn on_transfer_started(app: &mut App, start: StartedTransfer, tx: &mut Tx) {
    if let Some(previous) = app.transfer.ops.foreground.take() {
        app.transfer.ops.dock.remove(previous.op_id);
    }
    let docked = start
        .backgroundable
        .filter(|req_id| app.browser.background_file_operation(*req_id));
    let op_id = app.transfer.ops.allocate_id();
    let placement = if docked.is_some() {
        Placement::Dock
    } else {
        Placement::Dialog
    };
    let (icon, title, cancel_label) = if start.moving {
        (IconName::Folder, tr("Moving items"), tr("Cancel moving"))
    } else {
        (IconName::Copy, tr("Copying items"), tr("Cancel copying"))
    };
    let mut spec = OpSpec::new(icon, title, start.info);
    spec.subtitle = Some(tr("Cancelling will not undo completed changes"));
    spec.destination = (!start.destination.is_empty()).then_some(start.destination);
    spec.destination_description =
        (!start.destination_description.is_empty()).then_some(start.destination_description);
    spec.cancel_label = cancel_label;
    spec.transfer = true;
    spec.placement = placement;
    spec.reveal_after = reveal_after((start.total > 0).then_some(start.total));

    let counts = TransferCounts {
        total_items: start.total,
        ..Default::default()
    };
    let tracked = TrackedOp {
        op_id,
        request_id: docked.or(start.request_id),
        total_items: start.total,
        flushing: false,
        last_transfer: counts,
    };
    match docked {
        Some(req_id) => {
            app.transfer.ops.background.insert(req_id, tracked);
        }
        None => app.transfer.ops.foreground = Some(tracked),
    }
    let now = app.transfer.ops.last_clock;
    app.transfer.ops.dock.start(op_id, spec, tx);
    app.transfer
        .ops
        .dock
        .progress(op_id, Sample::new(Progress::Transfer(counts)), now, tx);
}

pub(super) fn on_transfer_progress(
    app: &mut App,
    bg: Option<OperationRequestId>,
    update: ProgressUpdate,
    tx: &mut Tx,
) -> bool {
    let now = app.transfer.ops.last_clock;
    let Some(tracked) = app.transfer.ops.tracked_mut(bg) else {
        return false;
    };
    let counts = TransferCounts {
        completed_items: update.completed_items,
        total_items: tracked.total_items,
        completed_files: update.completed_files,
        total_files: update.total_files,
        bytes: update.transferred_bytes,
        total_bytes: update.total_bytes,
    };
    tracked.last_transfer = counts;
    let op_id = tracked.op_id;
    let flushing = tracked.flushing;
    let mut sample = Sample::new(Progress::Transfer(counts));
    if let Some(current) = update.current_file.filter(|name| !name.is_empty()) {
        sample = sample.current(current);
    }
    if flushing {
        sample = sample.busy(tr("Writing to device…"));
    }
    app.transfer.ops.dock.progress(op_id, sample, now, tx)
}

pub(super) fn on_flushing(app: &mut App, bg: Option<OperationRequestId>, tx: &mut Tx) -> bool {
    let now = app.transfer.ops.last_clock;
    let Some(tracked) = app.transfer.ops.tracked_mut(bg) else {
        return false;
    };
    tracked.flushing = true;
    let op_id = tracked.op_id;
    let sample =
        Sample::new(Progress::Transfer(tracked.last_transfer)).busy(tr("Writing to device…"));
    app.transfer.ops.dock.progress(op_id, sample, now, tx)
}

pub(super) fn on_transfer_finished(
    app: &mut App,
    bg: Option<OperationRequestId>,
    moved_locations: &[Location],
) {
    if !moved_locations.is_empty() {
        clipboard::complete_cut_transfer(app, moved_locations);
    }
    if bg.is_none()
        && let Some(fg) = app.transfer.ops.foreground.take()
    {
        app.transfer.ops.dock.remove(fg.op_id);
    }
}

pub(super) fn on_transfer_completed(
    app: &mut App,
    bg: Option<OperationRequestId>,
    tx: &mut Tx,
) -> bool {
    let Some(tracked) = app.transfer.ops.take_tracked(bg) else {
        return false;
    };
    app.transfer
        .ops
        .dock
        .complete(tracked.op_id, tr("Copy complete"), tx)
}

pub(super) fn on_items_reported(app: &mut App, report: &TransferReport, tx: &mut Tx) -> bool {
    present_report(app, report, tx)
}

pub(super) fn on_operation_failed(
    app: &mut App,
    bg: Option<OperationRequestId>,
    message: String,
    report: &TransferReport,
) -> bool {
    if bg.is_none() {
        app.transfer.pending_reveal = None;
    }
    if let Some(tracked) = app.transfer.ops.take_tracked(bg) {
        app.transfer.ops.dock.remove(tracked.op_id);
    }
    let detail = outcome_detail(message, report);
    error_dialog::show(app, Error::new(tr("Unable to complete operation"), detail));
    true
}

pub(super) fn on_operation_cancelled(
    app: &mut App,
    bg: Option<OperationRequestId>,
    outcome: CancelledOutcome,
) -> bool {
    if bg.is_none() {
        app.transfer.pending_reveal = None;
    }
    if let Some(tracked) = app.transfer.ops.take_tracked(bg) {
        app.transfer.ops.dock.remove(tracked.op_id);
    }
    app.browser
        .refresh_after_cancellation(&outcome.affected_locations);
    let summary = cancelled_summary(outcome.completed, outcome.failed, outcome.not_attempted);
    let detail = outcome_detail(summary, &outcome.report);
    error_dialog::show(app, Error::new(tr("Operation cancelled"), detail));
    true
}

pub(super) fn on_cancel_pending(app: &mut App) -> bool {
    error_dialog::show(
        app,
        Error::new(
            tr("Transfer cancellation pending"),
            tr(
                "The device may still be writing. Wait for the transfer to finish or fail before starting another file operation. Do not unplug until you can safely eject it.",
            ),
        ),
    );
    true
}

pub(super) fn on_background_event(
    app: &mut App,
    request_id: OperationRequestId,
    event: BrowserEvent,
    tx: &mut Tx,
) -> bool {
    match event {
        BrowserEvent::TransferProgress {
            completed_items,
            completed_files,
            total_files,
            current_file,
            transferred_bytes,
            total_bytes,
        } => on_transfer_progress(
            app,
            Some(request_id),
            ProgressUpdate {
                completed_items,
                completed_files,
                total_files,
                current_file,
                transferred_bytes,
                total_bytes,
            },
            tx,
        ),
        BrowserEvent::FlushingToDevice => on_flushing(app, Some(request_id), tx),
        BrowserEvent::TransferFinished { moved_locations } => {
            on_transfer_finished(app, Some(request_id), &moved_locations);
            true
        }
        BrowserEvent::TransferCompleted => on_transfer_completed(app, Some(request_id), tx),
        BrowserEvent::ItemsReported { report } => on_items_reported(app, &report, tx),
        other => on_background_outcome(app, request_id, other, tx),
    }
}

fn on_background_outcome(
    app: &mut App,
    request_id: OperationRequestId,
    event: BrowserEvent,
    tx: &mut Tx,
) -> bool {
    match event {
        BrowserEvent::OperationFailed {
            message, report, ..
        } => on_operation_failed(app, Some(request_id), message, &report),
        BrowserEvent::OperationCancelled {
            completed,
            failed,
            not_attempted,
            affected_locations,
            report,
        } => on_operation_cancelled(
            app,
            Some(request_id),
            CancelledOutcome {
                completed,
                failed,
                not_attempted,
                affected_locations,
                report,
            },
        ),
        BrowserEvent::OperationRefreshRequired { depths } => {
            app.browser.refresh_operation_columns(&depths);
            false
        }
        BrowserEvent::TransferReveal {
            destination,
            locations,
        } => {
            super::reveal::apply(app, destination, locations, tx);
            true
        }
        _ => false,
    }
}

fn cancelled_summary(completed: usize, failed: usize, not_attempted: usize) -> String {
    rust_i18n::t!(
        "%{results}.\n\nCompleted changes were not reverted.",
        results = capitalize_first(list([
            count("items_completed", completed),
            count("items_failed", failed),
            count("items_not_attempted", not_attempted),
        ]))
    )
    .into_owned()
}

fn shown_name(name: &OsStr) -> String {
    name.to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect()
}

fn display_name(location: &Location) -> String {
    shown_name(&location.file_name().unwrap_or_default())
}

fn rename_heading(cause: RenameCause, single: bool) -> String {
    tr(match (cause, single) {
        (RenameCause::Alias, true) => {
            "The destination can\u{2019}t tell this name apart from another"
        }
        (RenameCause::Alias, false) => "The destination can\u{2019}t tell these names apart",
        (RenameCause::Taken, true) => "This name was already taken",
        (RenameCause::Taken, false) => "These names were already taken",
        (RenameCause::Invalid, true) => "The destination doesn\u{2019}t allow this name",
        (RenameCause::Invalid, false) => "The destination doesn\u{2019}t allow these names",
        (RenameCause::TooLong, true) => "This name was too long for the destination",
        (RenameCause::TooLong, false) => "These names were too long for the destination",
    })
}

fn renamed_line(item: &RenamedItem) -> String {
    rust_i18n::t!(
        "“%{name}” was saved as “%{new_name}”",
        name = display_name(&item.requested),
        new_name = display_name(&item.saved_as)
    )
    .into_owned()
}

fn skipped_line(item: &SkippedItem) -> String {
    special_line(&display_name(&item.location), item.kind)
}

fn kept_still_open(name: &str, folder: &str, holder: &Option<Holder>) -> String {
    match holder {
        Some(Holder::Program(program)) => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as a file in it is still open in %{program}",
            name = name,
            folder = folder,
            program = program
        )
        .into_owned(),
        Some(Holder::Strata) => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as a file in it is still in use by Strata",
            name = name,
            folder = folder
        )
        .into_owned(),
        None => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as a file in it is still open in another program",
            name = name,
            folder = folder
        )
        .into_owned(),
    }
}

fn kept_line(item: &KeptItem, in_trash: bool) -> String {
    let name = display_name(&item.location);
    let folder = || {
        item.location
            .parent()
            .map(|parent| display_name(&parent))
            .unwrap_or_default()
    };
    match &item.cause {
        KeptCause::Changed if in_trash => rust_i18n::t!(
            "“%{name}” changed during the restore and was left in the Trash",
            name = name
        )
        .into_owned(),
        KeptCause::Changed => rust_i18n::t!(
            "“%{name}” changed during the move and was left in “%{folder}”",
            name = name,
            folder = folder()
        )
        .into_owned(),
        KeptCause::ReplacementChanged => rust_i18n::t!(
            "“%{name}” was left in the Trash, as what replaced it may have changed since",
            name = name
        )
        .into_owned(),
        KeptCause::CopyGone => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as its copy was no longer where the move put it",
            name = name,
            folder = folder()
        )
        .into_owned(),
        KeptCause::StillOpen(holder) => kept_still_open(&name, &folder(), holder),
        KeptCause::Undeletable(error) => rust_i18n::t!(
            "Could not delete “%{name}”: %{error}",
            name = name,
            error = io_error_detail(&std::io::Error::from_raw_os_error(*error))
        )
        .into_owned(),
        KeptCause::Skipped(kind) => special_line(&name, *kind),
        KeptCause::OriginalNotRemoved(error) => rust_i18n::t!(
            "“%{name}” was replaced, but its original could not be removed from “%{kept}”: %{error}",
            name = name,
            kept = folder(),
            error = error
        )
        .into_owned(),
        KeptCause::NameNotRestored { saved_as, error } => rust_i18n::t!(
            "“%{new_name}” could not be renamed back to “%{name}”: %{error}",
            new_name = display_name(saved_as),
            name = name,
            error = error
        )
        .into_owned(),
    }
}

fn special_line(name: &str, kind: SpecialKind) -> String {
    match kind {
        SpecialKind::Pipe => rust_i18n::t!("“%{name}” (named pipe)", name = name),
        SpecialKind::Socket => rust_i18n::t!("“%{name}” (socket)", name = name),
        SpecialKind::Device => rust_i18n::t!("“%{name}” (device file)", name = name),
        SpecialKind::Symlink => rust_i18n::t!("“%{name}” (symbolic link)", name = name),
    }
    .into_owned()
}

fn bullets_from<T>(items: &[T], format_item: impl Fn(&T) -> String) -> String {
    let mut lines: Vec<String> = items
        .iter()
        .take(LISTED_ITEMS)
        .map(|item| format!("• {}", format_item(item)))
        .collect();
    if items.len() > LISTED_ITEMS {
        lines.push(count("and_more", items.len() - LISTED_ITEMS));
    }
    lines.join("\n")
}

fn renamed_sections(renamed: &[RenamedItem]) -> Vec<(String, Vec<String>, usize)> {
    const ORDER: [RenameCause; 4] = [
        RenameCause::Alias,
        RenameCause::Taken,
        RenameCause::Invalid,
        RenameCause::TooLong,
    ];
    ORDER
        .into_iter()
        .filter_map(|cause| {
            let matching: Vec<&RenamedItem> =
                renamed.iter().filter(|item| item.cause == cause).collect();
            (!matching.is_empty()).then(|| {
                let heading = rename_heading(cause, matching.len() == 1);
                let listed = matching
                    .iter()
                    .take(LISTED_ITEMS)
                    .map(|item| renamed_line(item))
                    .collect();
                (heading, listed, matching.len())
            })
        })
        .collect()
}

fn format_renamed_bullets(listed: &[String], total: usize) -> String {
    let mut lines: Vec<String> = listed.iter().map(|line| format!("• {line}")).collect();
    if total > LISTED_ITEMS {
        lines.push(count("and_more", total - LISTED_ITEMS));
    }
    lines.join("\n")
}

fn left_sections(report: &TransferReport) -> Vec<String> {
    let mut out = Vec::new();
    let (links, special): (Vec<_>, Vec<_>) = report
        .skipped
        .iter()
        .partition(|item| item.kind == SpecialKind::Symlink);
    if !special.is_empty() {
        let heading = tr(if special.len() == 1 {
            "This special file can\u{2019}t be copied and was left out"
        } else {
            "These special files can\u{2019}t be copied and were left out"
        });
        out.push(format!(
            "{heading}\n\n{}",
            bullets_from(&special, |s| skipped_line(s))
        ));
    }
    if !links.is_empty() {
        let heading = tr(if links.len() == 1 {
            "This symbolic link can\u{2019}t be stored at the destination and was left out"
        } else {
            "These symbolic links can\u{2019}t be stored at the destination and were left out"
        });
        out.push(format!(
            "{heading}\n\n{}",
            bullets_from(&links, |s| skipped_line(s))
        ));
    }
    if !report.kept.is_empty() {
        let heading = count("items_kept", report.kept.len());
        out.push(format!(
            "{heading}\n\n{}",
            bullets_from(&report.kept, |k| kept_line(k, false))
        ));
    }
    if !report.kept_in_trash.is_empty() {
        let heading = count("items_kept_in_trash", report.kept_in_trash.len());
        out.push(format!(
            "{heading}\n\n{}",
            bullets_from(&report.kept_in_trash, |k| kept_line(k, true))
        ));
    }
    out
}

fn outcome_detail(outcome: String, report: &TransferReport) -> String {
    let mut sections: Vec<String> = renamed_sections(&report.renamed)
        .into_iter()
        .map(|(heading, listed, total)| {
            format!("{heading}\n\n{}", format_renamed_bullets(&listed, total))
        })
        .collect();
    sections.extend(left_sections(report));
    if sections.is_empty() {
        outcome
    } else {
        format!("{outcome}\n\n{}", sections.join("\n\n"))
    }
}

fn present_report(app: &mut App, report: &TransferReport, tx: &mut Tx) -> bool {
    let mut renamed = renamed_sections(&report.renamed);
    let left = left_sections(report);
    if !left.is_empty() {
        let mut all: Vec<String> = renamed
            .into_iter()
            .map(|(h, l, total)| format!("{h}\n\n{}", format_renamed_bullets(&l, total)))
            .collect();
        all.extend(left);
        error_dialog::show(
            app,
            Error::new(tr("Completed with errors"), all.join("\n\n")),
        );
        return true;
    }
    if renamed.is_empty() {
        return false;
    }
    if renamed.len() == 1
        && let Some((heading, listed, 1)) = renamed.pop()
        && let [line] = listed.as_slice()
        && line.chars().count() <= NOTICE_CHARS
    {
        let notice = Notice::new(line.clone())
            .heading(heading)
            .joins()
            .shown_for(NOTICE_SECONDS);
        app.transfer.ops.dock.notice(notice, tx);
        return true;
    }
    let sections: Vec<String> = renamed_sections(&report.renamed)
        .into_iter()
        .map(|(h, l, total)| format!("{h}\n\n{}", format_renamed_bullets(&l, total)))
        .collect();
    let title = count("items_renamed", report.renamed.len());
    error_dialog::show(app, Error::new(title, sections.join("\n\n")));
    true
}
