// SPDX-License-Identifier: MIT

//! Background Jobs dashboard adapter (`src/ui/jobs.rs`) over `strata_fs::services::JobService`
//! and C16's `strata_components::progress_dock::jobs::Jobs`.

use std::{path::Path, rc::Rc, time::Duration};

use rune::{El, Tx};
use strata_components::progress_dock::jobs::{
    JobRow, JobStatus as UiJobStatus, Jobs, JobsIntent, JobsWords,
};
use strata_core::{
    i18n::{count, duration, integer, tr},
    model::ExecutionMode,
};
use strata_fs::services::{JobId, JobRequest, JobService, JobSnapshot, JobStatus};

use super::icons::action_icon;
use crate::{
    App,
    error_dialog::{self, Error},
    shell,
};

pub(super) const PUMP_INTERVAL: Duration = Duration::from_millis(120);
const ENQUEUE_PUMP_DELAY: Duration = Duration::from_millis(20);

pub(super) fn localized_words() -> JobsWords {
    JobsWords {
        jobs_running: Rc::new(|n| count("jobs_running", n)),
        queued: Rc::new(|n| rust_i18n::t!("%{count} queued", count = n).into_owned()),
        jobs_finished: Rc::new(|n| count("jobs_finished", n)),
        with_failures: Rc::new(|jobs| {
            rust_i18n::t!("%{jobs} · failures", jobs = jobs).into_owned()
        }),
        running: Rc::new(|n| rust_i18n::t!("%{count} running", count = n).into_owned()),
        finished: Rc::new(|n| rust_i18n::t!("%{count} finished", count = n).into_owned()),
        status: Rc::new(status_phrase),
        failed: Rc::new(|n| rust_i18n::t!("%{count} failed", count = n).into_owned()),
        hidden: Rc::new(|n| count("older_jobs_hidden", n)),
        duration: Rc::new(duration),
        integer: Rc::new(integer),
        title: tr("Jobs"),
        show: tr("Show background jobs"),
        none: tr("No background jobs."),
        no_output: tr("No output yet."),
        truncated: tr("Older output was discarded to bound memory"),
        clear: tr("Clear finished"),
        minimize: tr("Minimize"),
        cancel: tr("Cancel"),
        remove: tr("Remove"),
        details: tr("Details"),
        hide: tr("Hide"),
        dismiss: tr("Dismiss"),
    }
}

fn status_phrase(status: UiJobStatus, elapsed: &str) -> String {
    match status {
        UiJobStatus::Queued => tr("Queued"),
        UiJobStatus::Running => {
            rust_i18n::t!("Running for %{elapsed}", elapsed = elapsed).into_owned()
        }
        UiJobStatus::Cancelling => tr("Cancelling…"),
        UiJobStatus::Succeeded => {
            rust_i18n::t!("Done in %{elapsed}", elapsed = elapsed).into_owned()
        }
        UiJobStatus::Failed => {
            rust_i18n::t!("Failed after %{elapsed}", elapsed = elapsed).into_owned()
        }
        UiJobStatus::Cancelled => {
            rust_i18n::t!("Cancelled after %{elapsed}", elapsed = elapsed).into_owned()
        }
    }
}

pub(crate) fn build_footer_jobs(app: &App, layer: &mut Vec<El>) -> Option<El> {
    let mut builder = Jobs::new(
        "jobs",
        |app: &App| &app.custom_actions.jobs_state,
        |app: &mut App| &mut app.custom_actions.jobs_state,
        |app: &App| &app.custom_actions.job_rows,
    )
    .on_intent(handle_intent);
    if let Some(focus) = shell::view_focus(app) {
        builder = builder.return_focus(focus);
    }
    builder.build(app, &app.look, layer)
}

fn handle_intent(app: &mut App, intent: JobsIntent, tx: &mut Tx) {
    let service = super::shared_jobs(app);
    match intent {
        JobsIntent::Cancel(id) | JobsIntent::Remove(id) => {
            service.cancel(JobId(id));
        }
        JobsIntent::Dismiss(id) => {
            service.dismiss(JobId(id));
        }
        JobsIntent::ClearFinished => {
            service.clear_finished();
        }
    }
    sync_rows(app);
    close_when_emptied(app, tx);
}

/// GTK hides the Jobs button, and with it the open dashboard, once nothing is left to show
/// (`refresh_indicator` in `src/ui/jobs.rs`). C16 keeps an emptied list open on "No background
/// jobs.", so the adapter closes it and hands focus to the view, which GTK loses (finding
/// lgse/strata#2554).
fn close_when_emptied(app: &mut App, tx: &mut Tx) {
    let state = &mut app.custom_actions.jobs_state;
    if !state.is_open() || !app.custom_actions.job_rows.is_empty() {
        return;
    }
    state.close();
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, false);
    }
}

pub(super) fn enqueue_and_present(app: &mut App, request: JobRequest, tx: &mut Tx) {
    let service = super::shared_jobs(app);
    match service.enqueue(request) {
        Ok(id) => {
            tracing::debug!(?id, "a custom action's job was queued");
            app.custom_actions.featured_job = Some(id);
            app.custom_actions.jobs_state.open(tx);
            sync_rows(app);
            schedule_glib_pump(app, ENQUEUE_PUMP_DELAY);
        }
        Err(error) => {
            error_dialog::show(
                app,
                Error::new(tr("Unable to run action"), error.to_string()),
            );
        }
    }
}

pub(super) fn pump_from_timer(app: &mut App, tx: &mut Tx) -> bool {
    let Some(service) = app.custom_actions.job_service() else {
        return false;
    };
    service.pump();
    let changed = sync_rows(app);
    if has_active_jobs(&service) {
        tx.wake_after(PUMP_INTERVAL);
        schedule_glib_pump(app, PUMP_INTERVAL);
    }
    changed
}

pub(super) fn has_active_jobs(service: &JobService) -> bool {
    service.running_count() + service.queued_count() > 0
}

pub(super) fn schedule_glib_pump(app: &App, delay: Duration) {
    if app.custom_actions.pump_scheduled.replace(true) {
        return;
    }
    let flag = app.custom_actions.pump_scheduled.clone();
    let later = app.custom_actions.later.clone();
    glib::timeout_add_local_once(delay, move || {
        flag.set(false);
        later.then(pump_from_timer);
    });
}

pub(super) fn sync_rows(app: &mut App) -> bool {
    let Some(service) = app.custom_actions.job_service() else {
        return false;
    };
    let mut snapshots = service.snapshot();
    promote_featured(&mut snapshots, &mut app.custom_actions.featured_job);
    let clock = app.custom_actions.clock;
    let next: Vec<JobRow> = snapshots
        .iter()
        .map(|snapshot| snapshot_to_row(snapshot, clock))
        .collect();
    if app.custom_actions.job_rows == next {
        return false;
    }
    app.custom_actions.job_rows = next;
    true
}

fn promote_featured(snapshots: &mut Vec<JobSnapshot>, featured: &mut Option<JobId>) {
    let Some(id) = *featured else {
        return;
    };
    if let Some(index) = snapshots.iter().position(|snapshot| snapshot.id == id) {
        let row = snapshots.remove(index);
        snapshots.insert(0, row);
    } else {
        *featured = None;
    }
}

fn snapshot_to_row(snapshot: &JobSnapshot, clock: f64) -> JobRow {
    let status = convert_status(snapshot.status);
    let elapsed = snapshot.elapsed.as_secs_f64();
    let (started_at, ended_at) = if snapshot.is_active() {
        (clock - elapsed, None)
    } else {
        (0.0, Some(elapsed))
    };
    let (items, failed) = progress_counts(snapshot);
    JobRow {
        id: snapshot.id.0,
        name: snapshot.action_name.clone(),
        icon: action_icon(snapshot.icon.as_deref()),
        status,
        started_at,
        ended_at,
        message: snapshot.progress.message.clone(),
        items,
        failed,
        folder: compact_home(&snapshot.parent),
        fraction: snapshot.progress.fraction(snapshot.mode).map(|f| f as f32),
        output: snapshot.log.clone(),
        output_truncated: snapshot.log_truncated,
        error: snapshot.message.clone(),
        created: created_line(snapshot.created.len()),
    }
}

/// `RowWidgets::update` in `src/ui/jobs.rs`: shown only once the job reported something.
fn created_line(created: usize) -> Option<String> {
    (created > 0).then(|| {
        rust_i18n::t!(
            "Reported %{value1} created.",
            value1 = count("items", created)
        )
        .into_owned()
    })
}

fn progress_counts(snapshot: &JobSnapshot) -> (Option<(usize, usize)>, usize) {
    if snapshot.mode == ExecutionMode::PerItem && snapshot.progress.total_items > 0 {
        return (
            Some((
                snapshot.progress.completed_items,
                snapshot.progress.total_items,
            )),
            snapshot.progress.failed_items,
        );
    }
    let script_counts = snapshot
        .progress
        .script
        .as_ref()
        .and_then(|script| Some((script.completed, script.total.filter(|&total| total > 0)?)));
    (script_counts, 0)
}

fn convert_status(status: JobStatus) -> UiJobStatus {
    match status {
        JobStatus::Queued => UiJobStatus::Queued,
        JobStatus::Running => UiJobStatus::Running,
        JobStatus::Cancelling => UiJobStatus::Cancelling,
        JobStatus::Succeeded => UiJobStatus::Succeeded,
        JobStatus::Failed => UiJobStatus::Failed,
        JobStatus::Cancelled => UiJobStatus::Cancelled,
    }
}

fn compact_home(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => tr("Home"),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
