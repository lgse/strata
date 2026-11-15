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
    i18n::{capitalize_first, count, list, quoted, tr},
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

fn kept_changed_line(name: &str, folder: &str, in_trash: bool) -> String {
    if in_trash {
        rust_i18n::t!(
            "“%{name}” changed during the restore and was left in the Trash",
            name = name
        )
        .into_owned()
    } else {
        rust_i18n::t!(
            "“%{name}” changed during the move and was left in “%{folder}”",
            name = name,
            folder = folder
        )
        .into_owned()
    }
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

fn kept_failure_line(name: &str, folder: &str, cause: &KeptCause) -> Option<String> {
    match cause {
        KeptCause::Undeletable(error) => Some(
            rust_i18n::t!(
                "Could not delete “%{name}”: %{error}",
                name = name,
                error = io_error_detail(&std::io::Error::from_raw_os_error(*error))
            )
            .into_owned(),
        ),
        KeptCause::Skipped(kind) => Some(special_line(name, *kind)),
        KeptCause::OriginalNotRemoved(error) => Some(
            rust_i18n::t!(
                "“%{name}” was replaced, but its original could not be removed from “%{kept}”: %{error}",
                name = name,
                kept = folder,
                error = error
            )
            .into_owned(),
        ),
        KeptCause::NameNotRestored { saved_as, error } => Some(
            rust_i18n::t!(
                "“%{new_name}” could not be renamed back to “%{name}”: %{error}",
                new_name = display_name(saved_as),
                name = name,
                error = error
            )
            .into_owned(),
        ),
        _ => None,
    }
}

fn kept_line(item: &KeptItem, in_trash: bool) -> String {
    let name = display_name(&item.location);
    let folder = item
        .location
        .parent()
        .map(|parent| display_name(&parent))
        .unwrap_or_default();
    if let Some(line) = kept_failure_line(&name, &folder, &item.cause) {
        return line;
    }
    match &item.cause {
        KeptCause::Changed => kept_changed_line(&name, &folder, in_trash),
        KeptCause::ReplacementChanged => rust_i18n::t!(
            "“%{name}” was left in the Trash, as what replaced it may have changed since",
            name = name
        )
        .into_owned(),
        KeptCause::CopyGone => rust_i18n::t!(
            "“%{name}” was left in “%{folder}”, as its copy was no longer where the move put it",
            name = name,
            folder = folder
        )
        .into_owned(),
        KeptCause::StillOpen(holder) => kept_still_open(&name, &folder, holder),
        _ => String::new(),
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

struct Lines {
    listed: Vec<String>,
    count: usize,
    note: Option<String>,
}

impl Lines {
    fn new() -> Self {
        Self {
            listed: Vec::new(),
            count: 0,
            note: None,
        }
    }

    fn of<T>(items: impl IntoIterator<Item = T>, line: impl Fn(T) -> String) -> Self {
        let mut lines = Self::new();
        for item in items {
            lines.push(|| line(item));
        }
        lines
    }

    fn push(&mut self, line: impl FnOnce() -> String) {
        if self.count < LISTED_ITEMS {
            self.listed.push(line());
        }
        self.count += 1;
    }
}

fn bullets(lines: &Lines) -> String {
    let mut text = lines
        .listed
        .iter()
        .map(|line| format!("• {line}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    if lines.count > LISTED_ITEMS {
        text.push_str("\n\n");
        text.push_str(&count("more_items", lines.count - LISTED_ITEMS));
    }
    if let Some(note) = &lines.note {
        text.push_str("\n\n");
        text.push_str(note);
    }
    text
}

fn renamed_sections(renamed: &[RenamedItem]) -> Vec<(String, Lines)> {
    let mut sections = Vec::<(RenameCause, Lines)>::new();
    for item in renamed {
        let index = match sections.iter().position(|(cause, _)| *cause == item.cause) {
            Some(index) => index,
            None => {
                sections.push((item.cause, Lines::new()));
                sections.len() - 1
            }
        };
        sections[index].1.push(|| renamed_line(item));
    }
    sections
        .into_iter()
        .map(|(cause, lines)| (rename_heading(cause, lines.count == 1), lines))
        .collect()
}

fn sections(sections: Vec<(String, Lines)>) -> String {
    sections
        .into_iter()
        .map(|(heading, lines)| format!("{heading}\n\n{}", bullets(&lines)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn kept_undo_note(report: &TransferReport) -> Option<String> {
    if report.kept_out_of_undo.is_empty() {
        return None;
    }
    let left_out = Lines::of(&report.kept_out_of_undo, |item| quoted(&display_name(item)));
    let intro = tr(if report.kept_out_of_undo.len() == 1 {
        "Undo can\u{2019}t move this item back, as part of it stayed in its original folder."
    } else {
        "Undo can\u{2019}t move these items back, as part of each stayed in its original folder."
    });
    Some(format!("{intro}\n\n{}", bullets(&left_out)))
}

fn left_sections(report: &TransferReport) -> Vec<(String, Lines)> {
    let mut left = Vec::new();
    let (links, special): (Vec<_>, Vec<_>) = report
        .skipped
        .iter()
        .partition(|item| item.kind == SpecialKind::Symlink);
    for (skipped, one, several) in [
        (
            special,
            "This special file can\u{2019}t be copied and was left out",
            "These special files can\u{2019}t be copied and were left out",
        ),
        (
            links,
            "This symbolic link can\u{2019}t be stored at the destination and was left out",
            "These symbolic links can\u{2019}t be stored at the destination and were left out",
        ),
    ] {
        if !skipped.is_empty() {
            let heading = tr(if skipped.len() == 1 { one } else { several });
            left.push((heading, Lines::of(skipped, skipped_line)));
        }
    }
    for (kept, heading, in_trash) in [
        (&report.kept, "items_kept", false),
        (&report.kept_in_trash, "items_kept_in_trash", true),
    ] {
        if !kept.is_empty() {
            let mut lines = Lines::of(kept, |item| kept_line(item, in_trash));
            if !in_trash {
                lines.note = kept_undo_note(report);
            }
            left.push((count(heading, lines.count), lines));
        }
    }
    left
}

fn outcome_detail(outcome: String, report: &TransferReport) -> String {
    let mut reported = renamed_sections(&report.renamed);
    reported.extend(left_sections(report));
    if reported.is_empty() {
        return outcome;
    }
    format!("{outcome}\n\n{}", sections(reported))
}

pub(super) fn present_report(app: &mut App, report: &TransferReport, tx: &mut Tx) -> bool {
    let mut renamed = renamed_sections(&report.renamed);
    let left = left_sections(report);
    if !left.is_empty() {
        renamed.extend(left);
        error_dialog::show(app, Error::partial_failure(sections(renamed)));
        return true;
    }
    let message = count("items_renamed", report.renamed.len());
    if renamed.len() > 1 {
        error_dialog::show(
            app,
            Error::information(
                message,
                tr("Names changed for different reasons"),
                sections(renamed),
            ),
        );
        return true;
    }
    let Some((heading, lines)) = renamed.pop() else {
        return false;
    };
    if let [line] = lines.listed.as_slice()
        && line.chars().count() <= NOTICE_CHARS
    {
        let notice = Notice::new(line.clone())
            .heading(heading)
            .joins()
            .shown_for(NOTICE_SECONDS);
        app.transfer.ops.dock.notice(notice, tx);
        return true;
    }
    error_dialog::show(app, Error::information(message, heading, bullets(&lines)));
    true
}
