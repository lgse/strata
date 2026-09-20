// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

use crate::model::{ActionDefinition, ErrorPolicy, ExecutionMode, expand_arguments};

use super::actions::{ActionHandle, ActionProgram};
use super::listeners::{ListenerGuard, Listeners};

#[cfg(test)]
mod tests;

pub const MAX_CONCURRENT_JOBS: usize = 2;
pub const MAX_HISTORY: usize = 20;
const MAX_CREATED_LOCATIONS: usize = 20;
pub const MAX_LOG_BYTES: usize = 64 * 1024;
pub const MAX_MESSAGE_CHARS: usize = 512;
const CANCEL_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JobId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }

    pub const fn is_finished(self) -> bool {
        !self.is_active()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationSource {
    Selection,
    Background,
}

impl InvocationSource {
    pub const fn manifest_value(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::Background => "background",
        }
    }
}

#[derive(Clone, Debug)]
pub struct JobRequest {
    pub action: Rc<ActionHandle>,
    /// Targets captured when the action was invoked, in display order.
    pub inputs: Vec<PathBuf>,
    /// Folder the action was invoked from; also the default working directory.
    pub parent: PathBuf,
    pub source: InvocationSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobEnqueueError {
    EmptySelection,
    NotAbsolute(PathBuf),
    Unavailable(String),
}

impl std::fmt::Display for JobEnqueueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySelection => write!(formatter, "Select at least one item first"),
            Self::NotAbsolute(path) => write!(
                formatter,
                "“{}” is not an absolute path, so it cannot be passed to an action",
                path.display()
            ),
            Self::Unavailable(reason) => write!(formatter, "{reason}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ActionRunRequest {
    pub action: Rc<ActionHandle>,
    pub inputs: Vec<PathBuf>,
    pub parent: PathBuf,
    pub source: InvocationSource,
    /// 1-based position and total for a per-item invocation.
    pub position: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptProgress {
    pub completed: usize,
    pub total: Option<usize>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionRunEvent {
    Progress(ScriptProgress),
    /// Coalesced live tail of captured output.
    LogTail(String),
    Created(PathBuf),
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        log: String,
    },
    Failed(String),
}

pub type ActionEventSink = std::sync::Arc<dyn Fn(ActionRunEvent) + Send + Sync + 'static>;

pub type CancelHandle = Rc<dyn Fn()>;

pub trait ActionRunner {
    fn run(&self, request: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BoundedLog {
    text: String,
    truncated: bool,
}

impl BoundedLog {
    pub fn from_text(text: &str) -> Self {
        let mut log = Self::default();
        log.push(text);
        log
    }

    pub fn push(&mut self, chunk: &str) {
        self.text.push_str(chunk);
        self.trim();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    fn trim(&mut self) {
        if self.text.len() <= MAX_LOG_BYTES {
            return;
        }
        self.truncated = true;
        let mut cut = self.text.len() - MAX_LOG_BYTES;
        while cut < self.text.len() && !self.text.is_char_boundary(cut) {
            cut += 1;
        }
        self.text.drain(..cut);
        if let Some(newline) = self.text.find('\n') {
            self.text.drain(..=newline);
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JobProgress {
    /// Finished invocations. Per-item jobs count inputs; whole-selection jobs use 0 or 1.
    pub completed_items: usize,
    pub succeeded_items: usize,
    pub failed_items: usize,
    /// Planned invocations, always known for both modes.
    pub total_items: usize,
    pub script: Option<ScriptProgress>,
    pub message: Option<String>,
}

impl JobProgress {
    pub fn fraction(&self, mode: ExecutionMode) -> Option<f64> {
        match mode {
            ExecutionMode::WholeSelection => {
                let script = self.script.as_ref()?;
                let total = script.total?;
                (total > 0).then(|| (script.completed as f64 / total as f64).clamp(0.0, 1.0))
            }
            ExecutionMode::PerItem => {
                if self.total_items == 0 {
                    return None;
                }
                let within = match self.script.as_ref().and_then(|script| script.total) {
                    Some(total) if total > 0 => {
                        let script = self.script.as_ref().expect("script progress is present");
                        (script.completed as f64 / total as f64).clamp(0.0, 1.0)
                    }
                    _ => 0.0,
                };
                Some(
                    ((self.completed_items as f64 + within) / self.total_items as f64)
                        .clamp(0.0, 1.0),
                )
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct JobSnapshot {
    pub id: JobId,
    pub action_name: String,
    pub icon: Option<String>,
    pub mode: ExecutionMode,
    pub parent: PathBuf,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub log: String,
    pub log_truncated: bool,
    pub created: Vec<PathBuf>,
    pub message: Option<String>,
    pub elapsed: Duration,
}

impl JobSnapshot {
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

struct Job {
    id: JobId,
    action: Rc<ActionHandle>,
    definition: ActionDefinition,
    parent: PathBuf,
    source: InvocationSource,
    inputs: Vec<PathBuf>,
    cursor: usize,
    status: JobStatus,
    in_flight: bool,
    progress: JobProgress,
    log: BoundedLog,
    live_log: String,
    created: Vec<PathBuf>,
    message: Option<String>,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    run: u64,
    cancel: Option<CancelHandle>,
    cancel_requested: bool,
    cancelling_since: Option<Instant>,
}

impl Job {
    fn snapshot(&self) -> JobSnapshot {
        let elapsed = self.started_at.map_or(Duration::ZERO, |started| {
            self.finished_at
                .unwrap_or_else(Instant::now)
                .saturating_duration_since(started)
        });
        let mut log = self.log.clone();
        log.push(&self.live_log);
        JobSnapshot {
            id: self.id,
            action_name: self.definition.name.clone(),
            icon: self.definition.icon.clone(),
            mode: self.definition.run.mode,
            parent: self.parent.clone(),
            status: self.status,
            progress: self.progress.clone(),
            log: log.text().to_owned(),
            log_truncated: log.truncated(),
            created: self.created.clone(),
            message: self.message.clone(),
            elapsed,
        }
    }
}

struct JobMessage {
    job: JobId,
    run: u64,
    event: ActionRunEvent,
}

pub struct JobService {
    runner: Rc<dyn ActionRunner>,
    jobs: RefCell<Vec<Job>>,
    sender: Sender<JobMessage>,
    receiver: Receiver<JobMessage>,
    next_id: Cell<u64>,
    listeners: Rc<Listeners<Rc<dyn Fn()>>>,
}

impl JobService {
    pub fn new(runner: Rc<dyn ActionRunner>) -> Rc<Self> {
        let (sender, receiver) = mpsc::channel();
        Rc::new(Self {
            runner,
            jobs: RefCell::new(Vec::new()),
            sender,
            receiver,
            next_id: Cell::new(0),
            listeners: Rc::new(Listeners::new()),
        })
    }

    /// Execution starts at the next pump, not during enqueue.
    pub fn enqueue(&self, request: JobRequest) -> Result<JobId, JobEnqueueError> {
        if request.inputs.is_empty() {
            return Err(JobEnqueueError::EmptySelection);
        }
        if let Some(relative) = request.inputs.iter().find(|input| !input.is_absolute()) {
            return Err(JobEnqueueError::NotAbsolute(relative.clone()));
        }
        if let Some(reason) = request.action.unavailable_reason() {
            return Err(JobEnqueueError::Unavailable(reason.to_owned()));
        }
        let id = JobId(self.next_id.get().wrapping_add(1));
        self.next_id.set(id.0);
        let total_items = match request.action.definition.run.mode {
            ExecutionMode::PerItem => request.inputs.len(),
            ExecutionMode::WholeSelection => 1,
        };
        self.jobs.borrow_mut().push(Job {
            id,
            definition: request.action.definition.clone(),
            action: request.action,
            parent: request.parent,
            source: request.source,
            inputs: request.inputs,
            cursor: 0,
            status: JobStatus::Queued,
            in_flight: false,
            progress: JobProgress {
                total_items,
                ..JobProgress::default()
            },
            log: BoundedLog::default(),
            live_log: String::new(),
            created: Vec::new(),
            message: None,
            started_at: None,
            finished_at: None,
            run: 0,
            cancel: None,
            cancel_requested: false,
            cancelling_since: None,
        });
        self.notify();
        Ok(id)
    }

    pub fn pump(&self) {
        let applied = self.drain();
        let advanced = self.advance();
        let stalled = self.finalize_stalled_cancels();
        if applied || advanced || stalled {
            self.notify();
        }
    }

    fn drain(&self) -> bool {
        let mut jobs = self.jobs.borrow_mut();
        let mut changed = false;
        while let Ok(message) = self.receiver.try_recv() {
            let Some(job) = jobs.iter_mut().find(|job| job.id == message.job) else {
                continue;
            };
            if job.run != message.run || !job.in_flight {
                continue;
            }
            match message.event {
                ActionRunEvent::Progress(script) => {
                    job.progress.message = script.message.clone().map(sanitize_message);
                    job.progress.script = Some(ScriptProgress {
                        completed: script.completed,
                        total: script.total,
                        message: None,
                    });
                }
                ActionRunEvent::LogTail(text) => {
                    job.live_log = BoundedLog::from_text(&text).text().to_owned();
                }
                ActionRunEvent::Created(location) => {
                    if job.created.len() < MAX_CREATED_LOCATIONS && location.is_absolute() {
                        job.created.push(location);
                    }
                }
                ActionRunEvent::Exited { code, signal, log } => {
                    job.log.push(&log);
                    job.live_log.clear();
                    finish_invocation(job, code, signal);
                }
                ActionRunEvent::Failed(message) => {
                    job.log.push(&format!("{message}\n"));
                    job.live_log.clear();
                    finish_invocation(job, None, None);
                    if !job.cancel_requested {
                        job.message = Some(sanitize_message(message));
                    }
                }
            }
            changed = true;
        }
        changed
    }

    fn advance(&self) -> bool {
        let mut jobs = self.jobs.borrow_mut();
        let mut changed = false;
        let mut running = jobs
            .iter()
            .filter(|job| matches!(job.status, JobStatus::Running | JobStatus::Cancelling))
            .count();
        for index in 0..jobs.len() {
            let mode = jobs[index].definition.run.mode;
            let needs_dispatch = match jobs[index].status {
                JobStatus::Queued => {
                    if running >= MAX_CONCURRENT_JOBS {
                        continue;
                    }
                    true
                }
                JobStatus::Running if !jobs[index].in_flight => true,
                _ => false,
            };
            if !needs_dispatch {
                continue;
            }
            let chunk = match next_invocation(mode, &jobs[index].inputs, jobs[index].cursor) {
                Some(chunk) => chunk,
                None => {
                    jobs[index].status = if jobs[index].progress.failed_items > 0 {
                        JobStatus::Failed
                    } else {
                        JobStatus::Succeeded
                    };
                    jobs[index].finished_at = Some(Instant::now());
                    changed = true;
                    continue;
                }
            };
            jobs[index].status = JobStatus::Running;
            jobs[index].started_at.get_or_insert_with(Instant::now);
            jobs[index].cursor += chunk.inputs.len();
            jobs[index].run = jobs[index].run.wrapping_add(1);
            let request = ActionRunRequest {
                action: jobs[index].action.clone(),
                inputs: chunk.inputs,
                parent: jobs[index].parent.clone(),
                source: jobs[index].source,
                position: chunk.position,
            };
            let run = jobs[index].run;
            let id = jobs[index].id;
            let cancel = self.dispatch(&request, id, run);
            jobs[index].cancel = Some(cancel);
            jobs[index].in_flight = true;
            if jobs[index].progress.total_items == 0 {
                jobs[index].progress.total_items = chunk.position.map_or(1, |(_, total)| total);
            }
            running += usize::from(jobs[index].status == JobStatus::Running);
            changed = true;
        }
        changed
    }

    fn dispatch(&self, request: &ActionRunRequest, job: JobId, run: u64) -> CancelHandle {
        let sender = self.sender.clone();
        let sink: ActionEventSink = std::sync::Arc::new(move |event| {
            let _sent = sender.send(JobMessage { job, run, event });
        });
        self.runner.run(request, sink)
    }

    fn finalize_stalled_cancels(&self) -> bool {
        let now = Instant::now();
        let mut changed = false;
        for job in self.jobs.borrow_mut().iter_mut() {
            if job.status == JobStatus::Cancelling
                && cancel_deadline_reached(job.cancelling_since, now, CANCEL_DEADLINE)
            {
                job.cancel = None;
                job.in_flight = false;
                job.status = JobStatus::Cancelled;
                job.finished_at = Some(now);
                job.message = Some("Cancelled; the action did not stop cleanly".to_owned());
                changed = true;
            }
        }
        changed
    }

    pub fn cancel(&self, id: JobId) -> bool {
        let mut jobs = self.jobs.borrow_mut();
        let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
            return false;
        };
        match job.status {
            JobStatus::Queued => {
                job.status = JobStatus::Cancelled;
                job.finished_at = Some(Instant::now());
                job.message = Some("Removed before starting".to_owned());
            }
            JobStatus::Running => {
                job.status = JobStatus::Cancelling;
                job.cancel_requested = true;
                job.cancelling_since = Some(Instant::now());
                job.message = Some("Cancelling…".to_owned());
                if let Some(cancel) = job.cancel.take() {
                    cancel();
                }
            }
            JobStatus::Cancelling
            | JobStatus::Succeeded
            | JobStatus::Failed
            | JobStatus::Cancelled => return false,
        }
        drop(jobs);
        self.notify();
        true
    }

    pub fn dismiss(&self, id: JobId) -> bool {
        let mut jobs = self.jobs.borrow_mut();
        let before = jobs.len();
        jobs.retain(|job| job.id != id || job.status.is_active());
        let removed = jobs.len() != before;
        drop(jobs);
        if removed {
            self.notify();
        }
        removed
    }

    pub fn clear_finished(&self) -> bool {
        let mut jobs = self.jobs.borrow_mut();
        let before = jobs.len();
        jobs.retain(|job| job.status.is_active());
        let removed = jobs.len() != before;
        drop(jobs);
        if removed {
            self.notify();
        }
        removed
    }

    fn enforce_history_limit(&self) {
        let mut jobs = self.jobs.borrow_mut();
        let mut finished = jobs.iter().filter(|job| job.status.is_finished()).count();
        if finished <= MAX_HISTORY {
            return;
        }
        let mut removable: Vec<(Instant, JobId)> = jobs
            .iter()
            .filter(|job| job.status.is_finished())
            .map(|job| (job.finished_at.unwrap_or_else(Instant::now), job.id))
            .collect();
        removable.sort_by_key(|(finished_at, _)| *finished_at);
        for (_, id) in removable {
            if finished <= MAX_HISTORY {
                break;
            }
            if jobs.iter().any(|job| job.id == id) {
                jobs.retain(|job| job.id != id);
                finished -= 1;
            }
        }
    }

    pub fn running_count(&self) -> usize {
        self.jobs
            .borrow()
            .iter()
            .filter(|job| matches!(job.status, JobStatus::Running | JobStatus::Cancelling))
            .count()
    }

    pub fn queued_count(&self) -> usize {
        self.jobs
            .borrow()
            .iter()
            .filter(|job| job.status == JobStatus::Queued)
            .count()
    }

    pub fn finished_count(&self) -> usize {
        self.jobs
            .borrow()
            .iter()
            .filter(|job| job.status.is_finished())
            .count()
    }

    pub fn has_failures(&self) -> bool {
        self.jobs.borrow().iter().any(|job| {
            job.status == JobStatus::Failed
                || (job.status.is_finished() && job.progress.failed_items > 0)
        })
    }

    pub fn snapshot(&self) -> Vec<JobSnapshot> {
        let jobs = self.jobs.borrow();
        let mut active: Vec<_> = jobs
            .iter()
            .filter(|job| job.status.is_active())
            .map(Job::snapshot)
            .collect();
        active.sort_by_key(|job| match job.status {
            JobStatus::Running | JobStatus::Cancelling => 0,
            _ => 1,
        });
        let mut finished: Vec<_> = jobs
            .iter()
            .filter(|job| job.status.is_finished())
            .map(Job::snapshot)
            .collect();
        finished.sort_by_key(|job| std::cmp::Reverse(job.id));
        active.extend(finished);
        active
    }

    #[cfg(test)]
    pub fn snapshot_of(&self, id: JobId) -> Option<JobSnapshot> {
        self.jobs
            .borrow()
            .iter()
            .find(|job| job.id == id)
            .map(Job::snapshot)
    }

    pub(crate) fn observe(&self, callback: Rc<dyn Fn()>) -> ListenerGuard<Rc<dyn Fn()>> {
        self.listeners.add(callback)
    }

    fn notify(&self) {
        self.enforce_history_limit();
        self.listeners.notify(|callback| callback());
    }
}

struct InvocationChunk {
    inputs: Vec<PathBuf>,
    position: Option<(usize, usize)>,
}

fn next_invocation(
    mode: ExecutionMode,
    inputs: &[PathBuf],
    cursor: usize,
) -> Option<InvocationChunk> {
    match mode {
        ExecutionMode::WholeSelection => {
            (cursor == 0 && !inputs.is_empty()).then(|| InvocationChunk {
                inputs: inputs.to_vec(),
                position: None,
            })
        }
        ExecutionMode::PerItem => inputs.get(cursor).map(|input| InvocationChunk {
            inputs: vec![input.clone()],
            position: Some((cursor + 1, inputs.len())),
        }),
    }
}

fn finish_invocation(job: &mut Job, code: Option<i32>, signal: Option<i32>) {
    job.cancel = None;
    job.in_flight = false;
    job.progress.script = None;
    if job.cancel_requested {
        job.status = JobStatus::Cancelled;
        job.finished_at = Some(Instant::now());
        job.message.get_or_insert_with(|| "Cancelled".to_owned());
        return;
    }
    let success = code == Some(0) && signal.is_none();
    job.progress.completed_items += 1;
    if success {
        job.progress.succeeded_items += 1;
    } else {
        job.progress.failed_items += 1;
        job.message = Some(exit_failure_message(&job.definition, code, signal));
    }
    let failed = job.progress.failed_items > 0;
    let stop_on_error = job.definition.run.on_error == ErrorPolicy::Stop;
    let more_items = job.cursor < job.inputs.len();
    let keep_going = match job.definition.run.mode {
        ExecutionMode::WholeSelection => false,
        ExecutionMode::PerItem => more_items && !(failed && stop_on_error),
    };
    if keep_going {
        return;
    }
    job.finished_at = Some(Instant::now());
    job.status = if failed {
        JobStatus::Failed
    } else {
        JobStatus::Succeeded
    };
}

fn cancel_deadline_reached(
    cancelling_since: Option<Instant>,
    now: Instant,
    deadline: Duration,
) -> bool {
    cancelling_since.is_some_and(|since| now.saturating_duration_since(since) >= deadline)
}

fn exit_failure_message(
    definition: &ActionDefinition,
    code: Option<i32>,
    signal: Option<i32>,
) -> String {
    let detail = match (code, signal) {
        (Some(code), _) => format!("exited with status {code}"),
        (None, Some(signal)) => format!("was stopped by signal {signal}"),
        (None, None) => "could not be started".to_owned(),
    };
    format!("{} {detail}", definition.name)
}

fn sanitize_message(message: String) -> String {
    let cleaned: String = message
        .chars()
        .filter(|character| *character == '\n' || !character.is_control())
        .collect();
    let mut trimmed = cleaned.trim().to_owned();
    if trimmed.chars().count() > MAX_MESSAGE_CHARS {
        trimmed = trimmed.chars().take(MAX_MESSAGE_CHARS).collect();
        trimmed.push('…');
    }
    trimmed
}

pub fn expand_command_arguments(
    program: &ActionProgram,
    inputs: &[PathBuf],
    parent: &Path,
) -> Result<Vec<OsString>, String> {
    match program {
        ActionProgram::Command { arguments, .. } => {
            expand_arguments(arguments, inputs, parent).map_err(|error| error.to_string())
        }
        ActionProgram::Script { .. } => Ok(Vec::new()),
    }
}
