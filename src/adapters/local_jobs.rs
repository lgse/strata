// SPDX-License-Identifier: MIT

use std::{
    ffi::OsString,
    fs,
    io::{self, Read},
    os::unix::{fs::DirBuilderExt, fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdout, Command, Stdio},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process, kill_process_group};

use crate::model::{ExecutionMode, InterpreterFamily, WorkingDirectory};
use crate::services::{
    ActionEventSink, ActionHandle, ActionProgram, ActionRunEvent, ActionRunRequest, ActionRunner,
    CancelHandle, InvocationSource, ScriptProgress, expand_command_arguments,
};

#[cfg(test)]
mod tests;

const PROGRESS_POLL: Duration = Duration::from_millis(100);
const LOG_TAIL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_PROGRESS_LINE_BYTES: usize = 4096;
const MAX_PROGRESS_EVENTS: usize = 20_000;
const MAX_PROGRESS_UNITS: usize = 1_000_000;
const MAX_PROGRESS_MESSAGE_CHARS: usize = 512;
const MAX_PROGRESS_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PATHS_BYTES: usize = 4 * 1024 * 1024;
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
const REAP_DEADLINE: Duration = Duration::from_secs(5);
const LIVE_LOG_BYTES: usize = 8 * 1024;

const ENV_CONTEXT: &str = "STRATA_ACTION_CONTEXT";
const ENV_PATHS: &str = "STRATA_ACTION_PATHS";
const ENV_PARENT: &str = "STRATA_ACTION_PARENT";
const ENV_DIRECTORY: &str = "STRATA_ACTION_DIR";
const ENV_PROGRESS: &str = "STRATA_ACTION_PROGRESS";
const ENV_RUN_DIRECTORY: &str = "STRATA_ACTION_RUN_DIR";
const ENV_VERSION: &str = "STRATA_ACTION_VERSION";
const ENV_MODE: &str = "STRATA_ACTION_MODE";
const ENV_SOURCE: &str = "STRATA_ACTION_SOURCE";
const ENV_COUNT: &str = "STRATA_ACTION_COUNT";
const ENV_POSITION: &str = "STRATA_ACTION_POSITION";
const ENV_ACTION_ID: &str = "STRATA_ACTION_ID";

const PYTHON_HELPER: &str = include_str!("../../data/actions/strata_actions.py");
const PYTHON_HELPER_FILE: &str = "strata_actions.py";

pub(crate) struct LocalActionRunner {
    runtime_root: PathBuf,
}

impl LocalActionRunner {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            runtime_root: runtime_root(),
        })
    }

    #[cfg(test)]
    pub(crate) fn at(runtime_root: PathBuf) -> Rc<Self> {
        Rc::new(Self { runtime_root })
    }
}

fn runtime_root() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("strata")
}

impl ActionRunner for LocalActionRunner {
    fn run(&self, request: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
        let action = request.action.as_ref().clone();
        let Some(program) = action.program().cloned() else {
            let reason = action
                .unavailable_reason()
                .unwrap_or("This action cannot run on this system")
                .to_owned();
            return failed_immediately(sink, reason);
        };
        let run_directory = match create_run_directory(&self.runtime_root) {
            Ok(directory) => directory,
            Err(error) => {
                return failed_immediately(
                    sink,
                    format!("Unable to prepare a private working directory: {error}"),
                );
            }
        };

        let cancellation = Arc::new(Cancellation::default());
        let worker_cancel = cancellation.clone();
        let worker_sink = sink.clone();
        let context = RunContext {
            inputs: request.inputs.clone(),
            parent: request.parent.clone(),
            source: request.source,
            position: request.position,
            run_directory: run_directory.clone(),
        };
        let spawned = thread::Builder::new()
            .name("strata-action".to_owned())
            .spawn(move || {
                let result =
                    spawn_and_watch(&action, &program, &context, &worker_sink, &worker_cancel);
                let _ignored = fs::remove_dir_all(&context.run_directory);
                worker_sink(match result {
                    Ok(event) => event,
                    Err(error) => ActionRunEvent::Failed(error.to_string()),
                });
            });
        if let Err(error) = spawned {
            let _ignored = fs::remove_dir_all(&run_directory);
            return failed_immediately(sink, format!("Unable to start the action worker: {error}"));
        }

        Rc::new(move || cancellation.cancel())
    }
}

fn failed_immediately(sink: ActionEventSink, reason: String) -> CancelHandle {
    sink(ActionRunEvent::Failed(reason));
    Rc::new(|| {})
}

#[derive(Default)]
struct Cancellation {
    cancelled: std::sync::atomic::AtomicBool,
    pid: Mutex<Option<i32>>,
}

impl Cancellation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(pid) = self.process_group() {
            let _ignored = kill_process_group(pid, Signal::TERM);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn process_group(&self) -> Option<Pid> {
        let pid = self.pid.lock().ok().and_then(|pid| *pid)?;
        Pid::from_raw(pid)
    }

    fn remember_pid(&self, pid: i32) {
        if let Ok(mut slot) = self.pid.lock() {
            *slot = Some(pid);
        }
    }

    fn forget_pid(&self) {
        if let Ok(mut slot) = self.pid.lock() {
            *slot = None;
        }
    }
}

struct RunContext {
    inputs: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
    position: Option<(usize, usize)>,
    run_directory: PathBuf,
}

impl RunContext {
    fn working_directory(&self, action_directory: &Path, mode: WorkingDirectory) -> PathBuf {
        match mode {
            WorkingDirectory::Parent => self.parent.clone(),
            WorkingDirectory::Home => gtk::glib::home_dir(),
            WorkingDirectory::Action => action_directory.to_path_buf(),
        }
    }
}

struct InvocationFiles {
    context: PathBuf,
    paths: PathBuf,
    parent: PathBuf,
    progress: PathBuf,
}

fn spawn_and_watch(
    action: &ActionHandle,
    program: &ActionProgram,
    context: &RunContext,
    sink: &ActionEventSink,
    cancellation: &Cancellation,
) -> io::Result<ActionRunEvent> {
    let files = write_invocation_files(action, program, context)?;
    let mut command = build_command(action, context, &files)?;
    let working_directory =
        context.working_directory(&action.directory, action.definition.run.working_directory);
    command.current_dir(&working_directory);
    let mut child = command
        .spawn()
        .map_err(|error| io::Error::other(describe_spawn_failure(program, &error)))?;
    cancellation.remember_pid(child.id() as i32);
    let mut output = CapturedOutput {
        stdout: child.stdout.take(),
        stderr: child.stderr.take(),
        live: LiveOutput::default(),
    };
    let result = output
        .make_nonblocking()
        .and_then(|()| watch_process(&mut child, &files.progress, sink, cancellation, &mut output));
    if result.is_err() || cancellation.is_cancelled() {
        stop_process_group(&mut child, cancellation);
    }
    cancellation.forget_pid();
    let exit = result?;
    output.drain()?;
    Ok(ActionRunEvent::Exited {
        code: exit.code,
        signal: exit.signal,
        log: output.live.snapshot(),
    })
}

struct Exit {
    code: Option<i32>,
    signal: Option<i32>,
}

fn watch_process(
    child: &mut Child,
    progress_path: &Path,
    sink: &ActionEventSink,
    cancellation: &Cancellation,
    output: &mut CapturedOutput,
) -> io::Result<Exit> {
    let mut progress = ProgressReader::open(progress_path);
    let mut events = 0usize;
    let mut last_tail = Instant::now();
    let mut termination_deadline: Option<Instant> = None;
    let mut kill_deadline: Option<Instant> = None;

    loop {
        output.drain()?;
        events += progress.drain(sink, MAX_PROGRESS_EVENTS.saturating_sub(events));

        if cancellation.is_cancelled() && termination_deadline.is_none() {
            termination_deadline = Some(Instant::now() + TERMINATION_GRACE);
            if let Some(group) = cancellation.process_group() {
                let _ignored = kill_process_group(group, Signal::TERM);
            }
        }
        if let Some(deadline) = termination_deadline
            && Instant::now() >= deadline
            && kill_deadline.is_none()
        {
            kill_deadline = Some(Instant::now() + REAP_DEADLINE);
            if let Some(group) = cancellation.process_group() {
                let _ignored = kill_process_group(group, Signal::KILL);
            }
            let _ignored = child.kill();
        }
        if let Some(deadline) = kill_deadline
            && Instant::now() >= deadline
        {
            // Uninterruptible I/O must not keep the worker alive indefinitely.
            if let Some(pid) = Pid::from_raw(child.id() as i32) {
                let _ignored = kill_process(pid, Signal::KILL);
            }
            return Ok(Exit {
                code: None,
                signal: Some(9),
            });
        }

        match child.try_wait()? {
            Some(status) => {
                progress.drain(sink, MAX_PROGRESS_EVENTS.saturating_sub(events));
                return Ok(exit_from_status(status));
            }
            None => {
                if last_tail.elapsed() >= LOG_TAIL_INTERVAL {
                    last_tail = Instant::now();
                    let tail = output.live.tail();
                    if !tail.is_empty() {
                        sink(ActionRunEvent::LogTail(tail));
                    }
                }
                thread::sleep(PROGRESS_POLL);
            }
        }
    }
}

fn exit_from_status(status: std::process::ExitStatus) -> Exit {
    use std::os::unix::process::ExitStatusExt;
    Exit {
        code: status.code(),
        signal: status.signal(),
    }
}

fn build_command(
    action: &ActionHandle,
    context: &RunContext,
    files: &InvocationFiles,
) -> io::Result<Command> {
    let program = action
        .program()
        .expect("availability was checked before spawning");
    let mut command = match program {
        ActionProgram::Script {
            interpreter,
            interpreter_arguments,
            script,
            family,
        } => {
            let mut command = Command::new(interpreter);
            command.args(interpreter_arguments);
            if *family == InterpreterFamily::Python {
                command.env("PYTHONPATH", helper_python_path(&context.run_directory));
                command.env("PYTHONUNBUFFERED", "1");
            }
            command.arg(script);
            command
        }
        ActionProgram::Command { .. } => {
            let mut command = Command::new(
                action
                    .program()
                    .and_then(|program| match program {
                        ActionProgram::Command { program, .. } => Some(program.clone()),
                        ActionProgram::Script { .. } => None,
                    })
                    .expect("command programs carry a program"),
            );
            command.args(
                expand_command_arguments(program, &context.inputs, &context.parent)
                    .map_err(io::Error::other)?,
            );
            command
        }
    };
    command
        .env(ENV_VERSION, "1")
        .env(ENV_ACTION_ID, action.id())
        .env(ENV_MODE, mode_value(action))
        .env(ENV_SOURCE, context.source.manifest_value())
        .env(ENV_COUNT, context.inputs.len().to_string())
        .env(ENV_CONTEXT, &files.context)
        .env(ENV_PATHS, &files.paths)
        .env(ENV_PARENT, &files.parent)
        .env(ENV_DIRECTORY, &action.directory)
        .env(ENV_PROGRESS, &files.progress)
        .env(ENV_RUN_DIRECTORY, &context.run_directory);
    if let Some((position, _)) = context.position {
        command.env(ENV_POSITION, position.to_string());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Cancellation must also reach wrappers and pipelines.
    command.process_group(0);
    Ok(command)
}

fn mode_value(action: &ActionHandle) -> &'static str {
    match action.definition.run.mode {
        ExecutionMode::PerItem => "per-item",
        ExecutionMode::WholeSelection => "whole-selection",
    }
}

fn write_invocation_files(
    action: &ActionHandle,
    program: &ActionProgram,
    context: &RunContext,
) -> io::Result<InvocationFiles> {
    let directory = &context.run_directory;
    let mut paths = Vec::new();
    for input in &context.inputs {
        let bytes = path_bytes(input)?;
        paths.extend_from_slice(&bytes);
        paths.push(0);
        if paths.len() > MAX_PATHS_BYTES {
            return Err(io::Error::other(
                "Too many selected paths for one action invocation",
            ));
        }
    }
    write_private(&directory.join("paths"), &paths)?;
    write_private(&directory.join("parent"), &path_bytes(&context.parent)?)?;
    write_private(&directory.join("progress.jsonl"), b"")?;
    if matches!(
        program,
        ActionProgram::Script {
            family: InterpreterFamily::Python,
            ..
        }
    ) {
        write_private(
            &directory.join(PYTHON_HELPER_FILE),
            PYTHON_HELPER.as_bytes(),
        )?;
    }
    write_private(
        &directory.join("context.json"),
        invocation_context_json(action, context).as_bytes(),
    )?;
    Ok(InvocationFiles {
        context: directory.join("context.json"),
        paths: directory.join("paths"),
        parent: directory.join("parent"),
        progress: directory.join("progress.jsonl"),
    })
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    crate::storage::atomic_write(path, contents)
}

fn helper_python_path(run_directory: &Path) -> OsString {
    let mut value = run_directory.as_os_str().to_owned();
    if let Some(existing) = std::env::var_os("PYTHONPATH")
        && !existing.is_empty()
    {
        value.push(":");
        value.push(existing);
    }
    value
}

fn path_bytes(path: &Path) -> io::Result<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an empty path cannot be passed to an action",
        ));
    }
    Ok(bytes.to_vec())
}

// Native paths travel separately as bytes; JSON contains only display text.
fn invocation_context_json(action: &ActionHandle, context: &RunContext) -> String {
    let position = context
        .position
        .map(|(position, total)| format!("{position},{total}"))
        .unwrap_or_default();
    let fields = [
        ("version", "1".to_owned()),
        ("action_id", action.id().to_owned()),
        ("action_name", action.name().to_owned()),
        ("mode", mode_value(action).to_owned()),
        ("source", context.source.manifest_value().to_owned()),
        ("count", context.inputs.len().to_string()),
        ("position", position),
        ("paths_file", "paths".to_owned()),
        ("parent_file", "parent".to_owned()),
        (
            "parent_display",
            context.parent.to_string_lossy().into_owned(),
        ),
    ];
    let body = fields
        .iter()
        .map(|(key, value)| format!("  \"{key}\": {}", json_string(value)))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n{body}\n}}\n")
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

// The temporary-directory fallback may be shared with other users.
fn create_run_directory(runtime_root: &Path) -> io::Result<PathBuf> {
    let base = runtime_root.join("actions");
    match fs::symlink_metadata(&base) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                return Err(io::Error::other(format!(
                    "{} exists and is not a directory",
                    base.display()
                )));
            }
            if metadata.uid() != rustix::process::geteuid().as_raw() {
                return Err(io::Error::other(format!(
                    "{} is not owned by this user",
                    base.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&base)?;
        }
        Err(error) => return Err(error),
    }
    for _ in 0..64 {
        let candidate = base.join(format!("{}-{}", std::process::id(), next_run_sequence()));
        match fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(
        "Unable to allocate a private working directory",
    ))
}

fn next_run_sequence() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

fn describe_spawn_failure(program: &ActionProgram, error: &io::Error) -> String {
    let label = match program {
        ActionProgram::Script { script, .. } => script
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the script".to_owned()),
        ActionProgram::Command { program, .. } => program.to_string_lossy().into_owned(),
    };
    if error.kind() == io::ErrorKind::NotFound {
        return format!("“{label}” was not found");
    }
    format!("Unable to start “{label}”: {error}")
}

#[derive(Default)]
struct LiveOutput {
    text: Vec<u8>,
    truncated: bool,
}

impl LiveOutput {
    fn push(&mut self, chunk: &[u8]) {
        self.text.extend_from_slice(chunk);
        if self.text.len() > LIVE_LOG_BYTES {
            self.truncated = true;
            let cut = self.text.len() - LIVE_LOG_BYTES;
            self.text.drain(..cut);
            if let Some(newline) = self.text.iter().position(|byte| *byte == b'\n') {
                self.text.drain(..=newline);
            }
        }
    }

    fn tail(&self) -> String {
        String::from_utf8_lossy(&self.text).into_owned()
    }

    fn snapshot(&self) -> String {
        if self.truncated {
            format!("…\n{}", self.tail())
        } else {
            self.tail()
        }
    }
}

struct CapturedOutput {
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    live: LiveOutput,
}

impl CapturedOutput {
    fn make_nonblocking(&self) -> io::Result<()> {
        use std::os::fd::AsFd;
        for fd in [
            self.stdout.as_ref().map(AsFd::as_fd),
            self.stderr.as_ref().map(AsFd::as_fd),
        ]
        .into_iter()
        .flatten()
        {
            let flags = rustix::fs::fcntl_getfl(fd)?;
            rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
        }
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        for stream in [
            self.stdout.as_mut().map(|stream| stream as &mut dyn Read),
            self.stderr.as_mut().map(|stream| stream as &mut dyn Read),
        ]
        .into_iter()
        .flatten()
        {
            let mut bytes = [0; 4096];
            // A noisy stream must yield to cancellation and process-status polling.
            for _ in 0..16 {
                match stream.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => self.live.push(&bytes[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }
}

fn stop_process_group(child: &mut Child, cancellation: &Cancellation) {
    if let Some(group) = cancellation.process_group() {
        let _ignored = kill_process_group(group, Signal::KILL);
    }
    let _ignored = child.kill();
    let deadline = Instant::now() + REAP_DEADLINE;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        thread::sleep(PROGRESS_POLL);
    }
}

struct ProgressReader {
    file: Option<fs::File>,
    pending: String,
    offset: u64,
}

impl ProgressReader {
    fn open(path: &Path) -> Self {
        Self {
            file: fs::File::open(path).ok(),
            pending: String::new(),
            offset: 0,
        }
    }

    fn drain(&mut self, sink: &ActionEventSink, budget: usize) -> usize {
        if budget == 0 || self.offset >= MAX_PROGRESS_FILE_BYTES {
            return 0;
        }
        let mut chunk = String::new();
        if let Some(file) = self.file.as_mut() {
            let limit = (MAX_PROGRESS_FILE_BYTES - self.offset).min(64 * 1024);
            if file
                .by_ref()
                .take(limit)
                .read_to_string(&mut chunk)
                .is_err()
            {
                return 0;
            }
        } else {
            return 0;
        }
        self.offset += chunk.len() as u64;
        if chunk.is_empty() {
            return 0;
        }
        self.pending.push_str(&chunk);
        let mut applied = 0;
        while applied < budget {
            let Some(newline) = self.pending.find('\n') else {
                if self.pending.len() > MAX_PROGRESS_LINE_BYTES {
                    self.pending.clear();
                }
                break;
            };
            let line: String = self.pending.drain(..=newline).collect();
            if let Some(event) = parse_progress_line(line.trim_end()) {
                sink(event);
                applied += 1;
            }
        }
        applied
    }
}

fn parse_progress_line(line: &str) -> Option<ActionRunEvent> {
    if line.is_empty() || line.len() > MAX_PROGRESS_LINE_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    match value.get("event")?.as_str()? {
        "progress" => {
            let completed = bounded_units(value.get("processed")?)?;
            let total = value.get("total").and_then(bounded_units);
            let message = value
                .get("message")
                .and_then(|message| message.as_str())
                .map(|message| truncate(message, MAX_PROGRESS_MESSAGE_CHARS));
            Some(ActionRunEvent::Progress(ScriptProgress {
                completed,
                total,
                message,
            }))
        }
        "output" => {
            let path = value.get("path")?.as_str()?;
            if path.len() > MAX_PROGRESS_LINE_BYTES {
                return None;
            }
            let path = PathBuf::from(path);
            path.is_absolute().then_some(ActionRunEvent::Created(path))
        }
        _ => None,
    }
}

fn bounded_units(value: &serde_json::Value) -> Option<usize> {
    let number = value.as_u64()?;
    (number <= MAX_PROGRESS_UNITS as u64).then_some(number as usize)
}

fn truncate(text: &str, max_chars: usize) -> String {
    let cleaned: String = text
        .chars()
        .filter(|character| *character == '\n' || !character.is_control())
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let mut truncated: String = trimmed.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}
