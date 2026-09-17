// SPDX-License-Identifier: MIT

//! Runner tests.
//!
//! Each test builds its action through the real store, so interpreter
//! resolution, validation, and the invocation contract are exercised end to end.
//! Processes are real but short-lived; the cancellation test signals a sleeping
//! script immediately instead of waiting for it.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

use crate::model::{ActionDefinition, ActionRuntime, ArgumentToken, ErrorPolicy, ExecutionMode};
use crate::services::actions::ActionStore;
use crate::services::{
    ActionEventSink, ActionHandle, ActionRunEvent, ActionRunRequest, ActionRunner, ActionScript,
    ActionWriteRequest, CancelHandle, InvocationSource,
};

use super::*;
use crate::adapters::local_actions::LocalActionStore;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    store: Rc<LocalActionStore>,
    runner: Rc<LocalActionRunner>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("fixture");
        let root = directory.path().to_path_buf();
        let store = LocalActionStore::at(root.join("actions"));
        let runner = LocalActionRunner::at(root.join("runtime"));
        Self {
            _directory: directory,
            root,
            store,
            runner,
        }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    /// Creates an action through the store and loads it back.
    fn action(&self, id: &str, spec: &RunSpec<'_>, mode: ExecutionMode) -> Rc<ActionHandle> {
        let run = match spec {
            RunSpec::Script {
                runtime, file_name, ..
            } => {
                let runtime = match runtime {
                    ActionRuntime::Python => "python",
                    _ => "bash",
                };
                format!("runtime = \"{runtime}\"\nentrypoint = \"{file_name}\"\n")
            }
            RunSpec::Command { program, args } => {
                let arguments = args
                    .iter()
                    .map(|token| format!("\"{}\"", token_manifest_value(token)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("runtime = \"command\"\nprogram = \"{program}\"\nargs = [{arguments}]\n")
            }
        };
        let mode = match mode {
            ExecutionMode::PerItem => "per-item",
            ExecutionMode::WholeSelection => "whole-selection",
        };
        let on_error = match spec.on_error() {
            ErrorPolicy::Continue => "continue",
            ErrorPolicy::Stop => "stop",
        };
        let definition = ActionDefinition::parse(&format!(
            "schema_version = 1\nid = \"{id}\"\nname = \"Test\"\n\n[when]\n\n[run]\n{run}mode = \"{mode}\"\non_error = \"{on_error}\"\n"
        ))
        .expect("test definition is valid");
        let script = spec.script().map(|(file_name, contents)| ActionScript {
            file_name: file_name.to_owned(),
            contents: contents.to_owned(),
        });
        self.store
            .write(&ActionWriteRequest { definition, script })
            .expect("action writes");
        self.store
            .load()
            .get(id)
            .cloned()
            .unwrap_or_else(|| panic!("action {id} loads"))
    }
}

struct Outcome {
    events: Vec<ActionRunEvent>,
}

impl Outcome {
    fn ended(&self) -> (Option<i32>, Option<i32>, String) {
        for event in &self.events {
            match event {
                ActionRunEvent::Exited { code, signal, log } => {
                    return (*code, *signal, log.clone());
                }
                ActionRunEvent::Failed(message) => panic!("runner failed: {message}"),
                _ => {}
            }
        }
        panic!("the invocation never finished: {:?}", self.events);
    }

    fn progress(&self) -> Vec<ScriptProgress> {
        self.events
            .iter()
            .filter_map(|event| match event {
                ActionRunEvent::Progress(progress) => Some(progress.clone()),
                _ => None,
            })
            .collect()
    }

    fn created(&self) -> Vec<PathBuf> {
        self.events
            .iter()
            .filter_map(|event| match event {
                ActionRunEvent::Created(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }
}

/// One test action's run configuration.
#[derive(Clone)]
enum RunSpec<'a> {
    Script {
        runtime: ActionRuntime,
        file_name: &'a str,
        contents: &'a str,
    },
    Command {
        program: &'a str,
        args: Vec<ArgumentToken>,
    },
}

impl RunSpec<'_> {
    fn on_error(&self) -> ErrorPolicy {
        ErrorPolicy::Continue
    }

    fn script(&self) -> Option<(&str, &str)> {
        match self {
            Self::Script {
                file_name,
                contents,
                ..
            } => Some((file_name, contents)),
            Self::Command { .. } => None,
        }
    }
}

fn script<'a>(runtime: ActionRuntime, file_name: &'a str, contents: &'a str) -> RunSpec<'a> {
    RunSpec::Script {
        runtime,
        file_name,
        contents,
    }
}

fn command(program: &str, args: Vec<ArgumentToken>) -> RunSpec<'_> {
    RunSpec::Command { program, args }
}

/// Manifest text for one argument token, for building test manifests.
fn token_manifest_value(token: &ArgumentToken) -> String {
    match token {
        ArgumentToken::Literal(value) => value.clone(),
        ArgumentToken::Path => "{path}".to_owned(),
        ArgumentToken::Paths => "{paths}".to_owned(),
        ArgumentToken::Parent => "{parent}".to_owned(),
    }
}

fn request(action: Rc<ActionHandle>, inputs: &[PathBuf], parent: &Path) -> ActionRunRequest {
    ActionRunRequest {
        action,
        inputs: inputs.to_vec(),
        parent: parent.to_path_buf(),
        source: InvocationSource::Selection,
        position: None,
    }
}

fn channel_sink() -> (ActionEventSink, mpsc::Receiver<ActionRunEvent>) {
    let (sender, receiver) = mpsc::channel();
    let sink: ActionEventSink = std::sync::Arc::new(move |event: ActionRunEvent| {
        let _sent = sender.send(event);
    });
    (sink, receiver)
}

/// Runs one invocation and collects events until it ends.
fn run(runner: &Rc<LocalActionRunner>, request: ActionRunRequest) -> Outcome {
    let (sink, receiver) = channel_sink();
    let _cancel = runner.run(&request, sink);
    collect_until_end(receiver)
}

fn collect_until_end(receiver: mpsc::Receiver<ActionRunEvent>) -> Outcome {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(event) => {
                let ended = matches!(
                    event,
                    ActionRunEvent::Exited { .. } | ActionRunEvent::Failed(_)
                );
                events.push(event);
                if ended {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Outcome { events }
}

#[test]
fn runs_a_bash_script_and_captures_its_output() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "hello",
        &script(
            ActionRuntime::Bash,
            "run.sh",
            "#!/bin/bash\necho \"hello from $STRATA_ACTION_ID\"\n",
        ),
        ExecutionMode::WholeSelection,
    );
    if !action.is_available() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let outcome = run(&fixture.runner, request(action, &[], fixture.path()));
    let (code, signal, log) = outcome.ended();
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
    assert!(log.contains("hello from hello"), "{log}");
    let scratch = fixture.path().join("runtime/actions");
    assert!(
        !scratch
            .read_dir()
            .expect("scratch")
            .any(|entry| { entry.expect("entry").path().is_dir() }),
        "the private run directory is removed afterwards"
    );
}

#[test]
fn passes_selected_paths_as_bytes_and_lists_them_for_scripts() {
    use std::os::unix::ffi::OsStringExt;

    let fixture = Fixture::new();
    let action = fixture.action(
        "paths",
        &script(
            ActionRuntime::Python,
            "main.py",
            "#!/usr/bin/env python3\nfrom strata_actions import context\nctx = context()\nprint(repr(ctx.paths[0].encode('utf-8', 'surrogateescape')))\nprint(ctx.mode, ctx.source, ctx.count)\nprint(ctx.total)\n",
        ),
        ExecutionMode::WholeSelection,
    );
    if !action.is_available() {
        eprintln!("skipping: python3 is not installed");
        return;
    }
    let awkward = fixture
        .path()
        .join(OsString::from_vec(b"na\xffme with space.png".to_vec()));
    fs::write(&awkward, b"x").expect("fixture file");
    let outcome = run(
        &fixture.runner,
        request(action, std::slice::from_ref(&awkward), fixture.path()),
    );
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    assert!(
        log.contains("na\\xffme with space.png"),
        "the helper preserves bytes that are not valid UTF-8: {log}"
    );
    assert!(log.contains("whole-selection selection 1"), "{log}");
}

#[test]
fn reports_script_progress_and_created_locations() {
    let fixture = Fixture::new();
    let created = fixture.path().join("out.txt");
    let action = fixture.action(
        "progress",
        &script(
            ActionRuntime::Python,
            "main.py",
            &format!(
                "#!/usr/bin/env python3\nfrom strata_actions import context\nctx = context()\nctx.progress(2, 5, 'halfway')\nctx.output({})\nctx.output('relative.txt')\nprint('done')\n",
                serde_json::to_string(&created.display().to_string()).expect("path json")
            ),
        ),
        ExecutionMode::WholeSelection,
    );
    if !action.is_available() {
        eprintln!("skipping: python3 is not installed");
        return;
    }
    let outcome = run(&fixture.runner, request(action, &[], fixture.path()));
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    let progress = outcome.progress();
    assert_eq!(progress.len(), 1, "{:?}", outcome.events);
    assert_eq!(progress[0].completed, 2);
    assert_eq!(progress[0].total, Some(5));
    assert_eq!(progress[0].message.as_deref(), Some("halfway"));
    assert_eq!(
        outcome.created(),
        vec![created],
        "only absolute reported paths are accepted"
    );
}

#[test]
fn a_non_zero_exit_is_reported_with_its_status() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "fails",
        &script(
            ActionRuntime::Bash,
            "run.sh",
            "#!/bin/bash\necho 'bad' >&2\nexit 3\n",
        ),
        ExecutionMode::WholeSelection,
    );
    if !action.is_available() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let outcome = run(&fixture.runner, request(action, &[], fixture.path()));
    let (code, signal, log) = outcome.ended();
    assert_eq!(code, Some(3));
    assert_eq!(signal, None);
    assert!(log.contains("bad"), "{log}");
}

#[test]
fn commands_receive_expanded_arguments_without_a_shell() {
    let fixture = Fixture::new();
    let printer = fixture.path().join("argv.py");
    fs::write(&printer, "import sys\nprint('|'.join(sys.argv[1:]))\n").expect("printer");
    let action = fixture.action(
        "argv",
        &command(
            "python3",
            vec![
                ArgumentToken::Literal(printer.display().to_string()),
                ArgumentToken::Literal("--flag".to_owned()),
                ArgumentToken::Paths,
            ],
        ),
        ExecutionMode::WholeSelection,
    );
    if !action.is_available() {
        eprintln!("skipping: python3 is not installed");
        return;
    }
    // A name that would be command substitution in a shell.
    let awkward = fixture.path().join("$(touch pwned); rm -rf");
    let outcome = run(
        &fixture.runner,
        request(action, std::slice::from_ref(&awkward), fixture.path()),
    );
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    assert_eq!(log.trim_end(), format!("--flag|{}", awkward.display()));
    assert!(
        !fixture.path().join("pwned").exists(),
        "no shell expansion can happen"
    );
}

#[test]
fn an_unavailable_action_reports_why_instead_of_running() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "missing",
        &command("definitely-not-a-real-program-xyz", Vec::new()),
        ExecutionMode::WholeSelection,
    );
    assert!(!action.is_available());
    let outcome = run(&fixture.runner, request(action, &[], fixture.path()));
    match outcome.events.first() {
        Some(ActionRunEvent::Failed(message)) => {
            assert!(
                message.contains("definitely-not-a-real-program-xyz"),
                "{message}"
            );
        }
        other => panic!("expected a failure event, got {other:?}"),
    }
    assert!(
        !fixture.path().join("runtime/actions").exists(),
        "nothing is prepared for an action that cannot run"
    );
}

#[test]
fn cancellation_stops_a_running_script() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "sleepy",
        &script(
            ActionRuntime::Bash,
            "run.sh",
            "#!/bin/bash\necho started\nsleep 30\necho never\n",
        ),
        ExecutionMode::WholeSelection,
    );
    if !action.is_available() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let (sink, receiver) = channel_sink();
    let cancel: CancelHandle = fixture
        .runner
        .run(&request(action, &[], fixture.path()), sink);
    let started = Instant::now();
    let mut saw_output = false;
    let mut events = Vec::new();
    while started.elapsed() < EVENT_TIMEOUT {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(ActionRunEvent::LogTail(text)) if text.contains("started") => {
                saw_output = true;
                break;
            }
            Ok(ActionRunEvent::Exited { .. }) => break,
            Ok(event) => events.push(event),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(saw_output, "the script started: {events:?}");
    cancel();
    let outcome = collect_until_end(receiver);
    let (code, signal, log) = outcome.ended();
    assert!(
        signal.is_some(),
        "cancellation signals the process instead of letting it finish: code={code:?} log={log}"
    );
    assert!(!log.contains("never"), "the rest of the script did not run");
}

#[test]
fn progress_lines_are_validated() {
    assert!(parse_progress_line("").is_none());
    assert!(parse_progress_line("not json").is_none());
    assert!(parse_progress_line("{}").is_none());
    assert!(parse_progress_line("{\"event\":\"progress\"}").is_none());
    assert!(parse_progress_line("{\"event\":\"progress\",\"processed\":-1}").is_none());
    assert!(
        parse_progress_line("{\"event\":\"progress\",\"processed\":10000000}").is_none(),
        "absurd unit counts are refused"
    );
    assert!(parse_progress_line("{\"event\":\"output\",\"path\":\"relative\"}").is_none());
    assert!(parse_progress_line("{\"event\":\"unknown\"}").is_none());

    match parse_progress_line(
        "{\"event\":\"progress\",\"processed\":1,\"total\":2,\"message\":\"go\"}",
    ) {
        Some(ActionRunEvent::Progress(progress)) => {
            assert_eq!(progress.completed, 1);
            assert_eq!(progress.total, Some(2));
            assert_eq!(progress.message.as_deref(), Some("go"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    match parse_progress_line("{\"event\":\"output\",\"path\":\"/tmp/out.png\"}") {
        Some(ActionRunEvent::Created(path)) => assert_eq!(path, PathBuf::from("/tmp/out.png")),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn a_noisy_progress_file_is_bounded() {
    let fixture = tempfile::tempdir().expect("fixture");
    let progress = fixture.path().join("progress.jsonl");
    let mut lines = String::new();
    for index in 0..500 {
        lines.push_str(&format!(
            "{{\"event\":\"progress\",\"processed\":{index}}}\n"
        ));
    }
    // Garbage after valid JSON, and a line far beyond the accepted length.
    lines.push_str("{\"event\":\"progress\",\"processed\":1}\u{fffd}\n");
    lines.push_str(&format!("{}\n", "x".repeat(MAX_PROGRESS_LINE_BYTES * 2)));
    fs::write(&progress, lines).expect("progress file");

    let mut reader = ProgressReader::open(&progress);
    assert_eq!(
        reader.drain(&noop_sink(), 100),
        100,
        "the budget is respected"
    );
    let mut fresh = ProgressReader::open(&progress);
    assert_eq!(
        fresh.drain(&noop_sink(), MAX_PROGRESS_EVENTS),
        500,
        "only well-formed progress lines are counted"
    );
    assert_eq!(
        ProgressReader::open(&fixture.path().join("missing.jsonl")).drain(&noop_sink(), 10),
        0,
        "a missing progress file is harmless"
    );
}

#[test]
fn a_planted_scratch_base_is_refused() {
    let fixture = tempfile::tempdir().expect("fixture");
    // A symlink standing in for the base would redirect action scratch.
    let elsewhere = fixture.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("directory");
    std::os::unix::fs::symlink(&elsewhere, fixture.path().join("actions")).expect("symlink");
    assert!(
        create_run_directory(fixture.path()).is_err(),
        "a linked scratch base is refused instead of followed"
    );
    assert!(
        elsewhere.read_dir().expect("directory").next().is_none(),
        "nothing is written through the link"
    );

    // A plain file in the way is refused too.
    let file_fixture = tempfile::tempdir().expect("fixture");
    fs::write(file_fixture.path().join("actions"), b"not a directory").expect("file");
    assert!(create_run_directory(file_fixture.path()).is_err());
}

#[test]
fn the_private_run_directory_is_owner_only() {
    let fixture = tempfile::tempdir().expect("fixture");
    let directory = create_run_directory(fixture.path()).expect("run directory");
    assert!(directory.is_dir());
    let mode = fs::metadata(&directory)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "invocation scratch is private");
    assert_ne!(
        create_run_directory(fixture.path()).expect("second"),
        directory,
        "each invocation gets its own directory"
    );
}

fn noop_sink() -> ActionEventSink {
    std::sync::Arc::new(|_event: ActionRunEvent| {})
}
