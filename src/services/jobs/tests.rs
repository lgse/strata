// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::model::{ActionDefinition, ErrorPolicy, ExecutionMode};

use super::*;
use crate::services::actions::{ActionAvailability, ActionHandle, ActionProgram};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Recorded {
    action_id: String,
    inputs: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
    position: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
enum Behavior {
    Exit(i32),
    Scripted,
    HeldForCancel,
    Silent,
}

struct FakeRunner {
    behavior: Behavior,
    recorded: RefCell<Vec<Recorded>>,
    pending: RefCell<Vec<ActionEventSink>>,
    cancels: Rc<Cell<usize>>,
}

impl FakeRunner {
    fn new(behavior: Behavior) -> Rc<Self> {
        Rc::new(Self {
            behavior,
            recorded: RefCell::new(Vec::new()),
            pending: RefCell::new(Vec::new()),
            cancels: Rc::new(Cell::new(0)),
        })
    }

    fn recorded(&self) -> Vec<Recorded> {
        self.recorded.borrow().clone()
    }

    fn cancels(&self) -> usize {
        self.cancels.get()
    }

    fn finish_held(&self, code: Option<i32>, signal: Option<i32>) {
        let sink = self.pending.borrow_mut().pop();
        if let Some(sink) = sink {
            sink(ActionRunEvent::Exited {
                code,
                signal,
                log: "held\n".to_owned(),
            });
        }
    }
}

impl ActionRunner for FakeRunner {
    fn run(&self, request: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
        self.recorded.borrow_mut().push(Recorded {
            action_id: request.action.id().to_owned(),
            inputs: request.inputs.clone(),
            parent: request.parent.clone(),
            source: request.source,
            position: request.position,
        });
        match self.behavior {
            Behavior::Exit(code) => sink(ActionRunEvent::Exited {
                code: Some(code),
                signal: None,
                log: format!("exit {code}\n"),
            }),
            Behavior::Scripted => {
                sink(ActionRunEvent::Progress(ScriptProgress {
                    completed: 1,
                    total: Some(4),
                    message: Some("working".to_owned()),
                }));
                sink(ActionRunEvent::Exited {
                    code: Some(0),
                    signal: None,
                    log: "done\n".to_owned(),
                });
            }
            Behavior::HeldForCancel | Behavior::Silent => self.pending.borrow_mut().push(sink),
        }
        let cancels = self.cancels.clone();
        Rc::new(move || cancels.set(cancels.get() + 1))
    }
}

fn definition(mode: ExecutionMode, on_error: ErrorPolicy, runtime: &str) -> ActionDefinition {
    let mode = match mode {
        ExecutionMode::PerItem => "per-item",
        ExecutionMode::WholeSelection => "whole-selection",
    };
    let on_error = match on_error {
        ErrorPolicy::Continue => "continue",
        ErrorPolicy::Stop => "stop",
    };
    let run = match runtime {
        "command" => format!(
            "runtime = \"command\"\nprogram = \"true\"\nmode = \"{mode}\"\non_error = \"{on_error}\"\n"
        ),
        _ => format!(
            "runtime = \"bash\"\nentrypoint = \"run.sh\"\nmode = \"{mode}\"\non_error = \"{on_error}\"\n"
        ),
    };
    ActionDefinition::parse(&format!(
        "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n\n[when]\n\n[run]\n{run}"
    ))
    .expect("test definition is valid")
}

fn handle(
    id: &str,
    name: &str,
    mode: ExecutionMode,
    on_error: ErrorPolicy,
    runtime: &str,
) -> Rc<ActionHandle> {
    let mut definition = definition(mode, on_error, runtime);
    definition.id = id.to_owned();
    definition.name = name.to_owned();
    Rc::new(ActionHandle {
        definition,
        directory: PathBuf::from("/tmp/actions").join(id),
        availability: ActionAvailability::Available(ActionProgram::Command {
            program: OsString::from("true"),
            arguments: Vec::new(),
        }),
    })
}

fn unavailable_handle() -> Rc<ActionHandle> {
    Rc::new(ActionHandle {
        definition: definition(ExecutionMode::WholeSelection, ErrorPolicy::Continue, "bash"),
        directory: PathBuf::from("/tmp/actions/a1"),
        availability: ActionAvailability::Unavailable {
            reason: "The interpreter “python3” was not found".to_owned(),
        },
    })
}

fn request(action: Rc<ActionHandle>, inputs: &[&str]) -> JobRequest {
    JobRequest {
        action,
        inputs: inputs.iter().map(PathBuf::from).collect(),
        parent: PathBuf::from("/tmp/parent"),
        source: InvocationSource::Selection,
    }
}

fn settle(service: &JobService) {
    for _ in 0..64 {
        service.pump();
    }
}

#[test]
fn runs_at_most_two_jobs_and_queues_the_rest() {
    let runner = FakeRunner::new(Behavior::HeldForCancel);
    let service = JobService::new(runner.clone());
    for index in 0..3 {
        service
            .enqueue(request(
                handle(
                    &format!("a{index}"),
                    "Action",
                    ExecutionMode::WholeSelection,
                    ErrorPolicy::Continue,
                    "command",
                ),
                &["/tmp/one.txt"],
            ))
            .expect("job queues");
    }
    service.pump();
    assert_eq!(service.running_count(), MAX_CONCURRENT_JOBS);
    assert_eq!(service.queued_count(), 1);
    assert_eq!(runner.recorded().len(), MAX_CONCURRENT_JOBS);

    runner.finish_held(Some(0), None);
    settle(&service);
    assert_eq!(
        runner.recorded().len(),
        3,
        "the queued job starts once a slot frees"
    );
    assert_eq!(service.queued_count(), 0);
    assert!(
        service.running_count() <= MAX_CONCURRENT_JOBS,
        "concurrency stays bounded when a slot is reused"
    );
}

#[test]
fn per_item_jobs_iterate_one_input_at_a_time_and_report_positions() {
    let runner = FakeRunner::new(Behavior::Scripted);
    let service = JobService::new(runner.clone());
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Resize",
                ExecutionMode::PerItem,
                ErrorPolicy::Continue,
                "bash",
            ),
            &["/tmp/a.png", "/tmp/b.png", "/tmp/c.png"],
        ))
        .expect("job queues");
    settle(&service);

    let recorded = runner.recorded();
    assert_eq!(recorded.len(), 3);
    assert_eq!(
        recorded
            .iter()
            .map(|run| (run.inputs.clone(), run.position))
            .collect::<Vec<_>>(),
        vec![
            (vec![PathBuf::from("/tmp/a.png")], Some((1, 3))),
            (vec![PathBuf::from("/tmp/b.png")], Some((2, 3))),
            (vec![PathBuf::from("/tmp/c.png")], Some((3, 3))),
        ]
    );
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Succeeded);
    assert_eq!(snapshot.progress.completed_items, 3);
    assert_eq!(snapshot.progress.succeeded_items, 3);
    assert_eq!(snapshot.progress.failed_items, 0);
    assert_eq!(snapshot.progress.total_items, 3);
    assert!(!snapshot.is_active());
}

#[test]
fn whole_selection_jobs_run_once_with_every_input() {
    let runner = FakeRunner::new(Behavior::Exit(0));
    let service = JobService::new(runner.clone());
    service
        .enqueue(request(
            handle(
                "a1",
                "Checksums",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "bash",
            ),
            &["/tmp/a.bin", "/tmp/b.bin"],
        ))
        .expect("job queues");
    settle(&service);
    let recorded = runner.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].inputs,
        vec![PathBuf::from("/tmp/a.bin"), PathBuf::from("/tmp/b.bin")]
    );
    assert_eq!(recorded[0].position, None);
}

#[test]
fn continue_on_error_keeps_going_and_reports_partial_success() {
    let runner = FakeRunner::new(Behavior::Exit(3));
    let service = JobService::new(runner.clone());
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Convert",
                ExecutionMode::PerItem,
                ErrorPolicy::Continue,
                "bash",
            ),
            &["/tmp/a.png", "/tmp/b.png"],
        ))
        .expect("job queues");
    settle(&service);
    assert_eq!(runner.recorded().len(), 2, "both items are attempted");
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Failed);
    assert_eq!(snapshot.progress.completed_items, 2);
    assert_eq!(snapshot.progress.failed_items, 2);
    assert_eq!(snapshot.progress.succeeded_items, 0);
    assert!(
        snapshot
            .message
            .as_deref()
            .is_some_and(|message| message.contains("status 3")),
        "the failure names the exit status: {:?}",
        snapshot.message
    );
}

#[test]
fn stop_on_error_halts_after_the_first_failure() {
    let runner = FakeRunner::new(Behavior::Exit(1));
    let service = JobService::new(runner.clone());
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Convert",
                ExecutionMode::PerItem,
                ErrorPolicy::Stop,
                "bash",
            ),
            &["/tmp/a.png", "/tmp/b.png", "/tmp/c.png"],
        ))
        .expect("job queues");
    settle(&service);
    assert_eq!(runner.recorded().len(), 1);
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Failed);
    assert_eq!(snapshot.progress.completed_items, 1);
    assert_eq!(snapshot.progress.failed_items, 1);
}

#[test]
fn partial_success_is_reported_when_some_items_fail() {
    struct MixedRunner {
        calls: Cell<usize>,
    }
    impl ActionRunner for MixedRunner {
        fn run(&self, _: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
            let call = self.calls.get();
            self.calls.set(call + 1);
            sink(ActionRunEvent::Exited {
                code: Some(if call == 0 { 2 } else { 0 }),
                signal: None,
                log: format!("item {call}\n"),
            });
            Rc::new(|| {})
        }
    }
    let service = JobService::new(Rc::new(MixedRunner {
        calls: Cell::new(0),
    }));
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Convert",
                ExecutionMode::PerItem,
                ErrorPolicy::Continue,
                "bash",
            ),
            &["/tmp/a.png", "/tmp/b.png"],
        ))
        .expect("job queues");
    settle(&service);
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Failed);
    assert_eq!(snapshot.progress.succeeded_items, 1);
    assert_eq!(snapshot.progress.failed_items, 1);
    assert_eq!(
        snapshot.log, "item 0\nitem 1\n",
        "later items retain earlier output"
    );
}

#[test]
fn cancelling_a_queued_job_removes_it_without_running_it() {
    let runner = FakeRunner::new(Behavior::HeldForCancel);
    let service = JobService::new(runner.clone());
    let first = service
        .enqueue(request(
            handle(
                "a1",
                "One",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/a"],
        ))
        .expect("job queues");
    let second = service
        .enqueue(request(
            handle(
                "a2",
                "Two",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/b"],
        ))
        .expect("job queues");
    let third = service
        .enqueue(request(
            handle(
                "a3",
                "Three",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/c"],
        ))
        .expect("job queues");
    service.pump();
    assert!(service.cancel(third), "queued jobs can be removed");
    let snapshot = service.snapshot_of(third).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Cancelled);
    assert!(snapshot.message.is_some());
    assert!(
        !service.cancel(third),
        "an already-cancelled job is not cancellable"
    );
    assert_eq!(
        service.snapshot_of(first).map(|job| job.status),
        Some(JobStatus::Running)
    );
    assert_eq!(
        service.snapshot_of(second).map(|job| job.status),
        Some(JobStatus::Running)
    );
    runner.finish_held(Some(0), None);
    settle(&service);
    assert_eq!(
        runner.recorded().len(),
        2,
        "the removed job must never start"
    );
}

#[test]
fn cancelling_a_running_job_waits_for_the_runner_to_report_back() {
    let runner = FakeRunner::new(Behavior::HeldForCancel);
    let service = JobService::new(runner.clone());
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Convert",
                ExecutionMode::PerItem,
                ErrorPolicy::Continue,
                "bash",
            ),
            &["/tmp/a.png", "/tmp/b.png"],
        ))
        .expect("job queues");
    service.pump();
    assert_eq!(runner.recorded().len(), 1);
    assert!(service.cancel(id));
    assert_eq!(runner.cancels(), 1, "the runner's cancel handle is invoked");
    assert_eq!(
        service.running_count(),
        1,
        "cancelling jobs still occupy a slot"
    );
    assert_eq!(
        service.snapshot_of(id).map(|job| job.status),
        Some(JobStatus::Cancelling)
    );

    runner.finish_held(None, Some(15));
    settle(&service);
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Cancelled);
    assert_eq!(
        runner.recorded().len(),
        1,
        "cancellation must not continue to later inputs"
    );
    assert!(!snapshot.is_active());
}

#[test]
fn a_runner_that_ignores_cancellation_keeps_its_slot() {
    let runner = FakeRunner::new(Behavior::Silent);
    let service = JobService::new(runner.clone());
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Convert",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/a"],
        ))
        .expect("job queues");
    service.pump();
    assert!(service.cancel(id));
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Cancelling);
    assert_eq!(runner.cancels(), 1);
    service.pump();
    assert_eq!(service.running_count(), 1);
}

#[test]
fn rejects_relative_paths_empty_selections_and_unavailable_actions() {
    let runner = FakeRunner::new(Behavior::Exit(0));
    let service = JobService::new(runner);
    let action = handle(
        "a1",
        "One",
        ExecutionMode::WholeSelection,
        ErrorPolicy::Continue,
        "command",
    );
    assert_eq!(
        service.enqueue(request(action.clone(), &[])),
        Err(JobEnqueueError::EmptySelection)
    );
    assert_eq!(
        service.enqueue(JobRequest {
            action: action.clone(),
            inputs: vec![PathBuf::from("relative.txt")],
            parent: PathBuf::from("/tmp"),
            source: InvocationSource::Selection,
        }),
        Err(JobEnqueueError::NotAbsolute(PathBuf::from("relative.txt")))
    );
    assert!(matches!(
        service.enqueue(request(unavailable_handle(), &["/tmp/a"])),
        Err(JobEnqueueError::Unavailable(_))
    ));
}

#[test]
fn history_is_bounded_and_active_jobs_survive() {
    let service = JobService::new(FakeRunner::new(Behavior::Exit(0)));
    let mut ids = Vec::new();
    for index in 0..MAX_HISTORY + 4 {
        ids.push(
            service
                .enqueue(request(
                    handle(
                        &format!("a{index}"),
                        "One",
                        ExecutionMode::WholeSelection,
                        ErrorPolicy::Continue,
                        "command",
                    ),
                    &["/tmp/a"],
                ))
                .expect("job queues"),
        );
    }
    settle(&service);
    assert_eq!(service.running_count(), 0);
    assert_eq!(service.queued_count(), 0);
    assert_eq!(service.finished_count(), MAX_HISTORY);
    assert!(
        service.snapshot_of(ids[0]).is_none(),
        "the oldest finished job is evicted"
    );
    assert!(service.snapshot_of(*ids.last().expect("ids")).is_some());
}

#[test]
fn dismiss_and_clear_only_touch_finished_jobs() {
    let runner = FakeRunner::new(Behavior::HeldForCancel);
    let service = JobService::new(runner.clone());
    let active = service
        .enqueue(request(
            handle(
                "a1",
                "One",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/a"],
        ))
        .expect("job queues");
    let finished = service
        .enqueue(request(
            handle(
                "a2",
                "Two",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/b"],
        ))
        .expect("job queues");
    service.pump();
    runner.finish_held(Some(0), None);
    settle(&service);

    assert!(service.dismiss(finished));
    assert!(service.snapshot_of(finished).is_none());
    assert!(!service.dismiss(active), "active jobs are not dismissable");
    assert_eq!(
        service.snapshot_of(active).map(|job| job.status),
        Some(JobStatus::Running)
    );
    assert!(!service.clear_finished());
    assert!(service.snapshot_of(active).is_some());
}

#[test]
fn snapshots_order_active_jobs_before_finished_ones() {
    let runner = FakeRunner::new(Behavior::HeldForCancel);
    let service = JobService::new(runner.clone());
    let finished = service
        .enqueue(request(
            handle(
                "a1",
                "Finished",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/a"],
        ))
        .expect("job queues");
    service.pump();
    runner.finish_held(Some(0), None);
    settle(&service);
    let active = service
        .enqueue(request(
            handle(
                "a2",
                "Active",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/b"],
        ))
        .expect("job queues");
    service.pump();

    let snapshot = service.snapshot();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].id, active);
    assert!(snapshot[0].is_active());
    assert_eq!(snapshot[1].id, finished);
    assert!(snapshot[1].status.is_finished());
    assert_eq!(snapshot[1].action_name, "Finished");
}

#[test]
fn produces_script_reported_output_and_messages() {
    struct ScriptedRunner;
    impl ActionRunner for ScriptedRunner {
        fn run(&self, _: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
            sink(ActionRunEvent::Progress(ScriptProgress {
                completed: 3,
                total: Some(10),
                message: Some("converting".to_owned()),
            }));
            sink(ActionRunEvent::LogTail("halfway\n".to_owned()));
            sink(ActionRunEvent::Created(PathBuf::from("/tmp/out.png")));
            sink(ActionRunEvent::Created(PathBuf::from("relative.png")));
            sink(ActionRunEvent::Exited {
                code: Some(0),
                signal: None,
                log: "finished\n".to_owned(),
            });
            Rc::new(|| {})
        }
    }
    let service = JobService::new(Rc::new(ScriptedRunner));
    let id = service
        .enqueue(request(
            handle(
                "a1",
                "Convert",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "bash",
            ),
            &["/tmp/a.png"],
        ))
        .expect("job queues");
    settle(&service);
    let snapshot = service.snapshot_of(id).expect("job exists");
    assert_eq!(snapshot.status, JobStatus::Succeeded);
    assert_eq!(
        snapshot.created,
        vec![PathBuf::from("/tmp/out.png")],
        "only absolute reported locations are kept"
    );
    assert_eq!(snapshot.log, "finished\n");
    assert!(!snapshot.log_truncated);
}

#[test]
fn progress_fractions_are_honest_about_unknown_totals() {
    let mut progress = JobProgress {
        total_items: 4,
        completed_items: 1,
        ..JobProgress::default()
    };
    assert_eq!(
        progress.fraction(ExecutionMode::PerItem),
        Some(0.25),
        "per-item progress is measurable from item counts alone"
    );
    progress.script = Some(ScriptProgress {
        completed: 2,
        total: Some(4),
        message: None,
    });
    assert_eq!(progress.fraction(ExecutionMode::PerItem), Some(0.375));

    let whole = JobProgress {
        total_items: 1,
        script: None,
        ..JobProgress::default()
    };
    assert_eq!(
        whole.fraction(ExecutionMode::WholeSelection),
        None,
        "an unmodified command reports no percentage"
    );
    let reported = JobProgress {
        total_items: 1,
        script: Some(ScriptProgress {
            completed: 3,
            total: Some(10),
            message: None,
        }),
        ..JobProgress::default()
    };
    assert_eq!(reported.fraction(ExecutionMode::WholeSelection), Some(0.3));
    let zero_total = JobProgress {
        total_items: 1,
        script: Some(ScriptProgress {
            completed: 5,
            total: Some(0),
            message: None,
        }),
        ..JobProgress::default()
    };
    assert_eq!(zero_total.fraction(ExecutionMode::WholeSelection), None);
}

#[test]
fn logs_keep_the_newest_output_within_the_bound() {
    let mut log = BoundedLog::default();
    log.push("first line\n");
    for index in 0..MAX_LOG_BYTES {
        log.push(&format!("line {index}\n"));
    }
    assert!(log.text().len() <= MAX_LOG_BYTES);
    assert!(log.truncated());
    assert!(
        !log.text().starts_with("first line"),
        "the oldest output is dropped first"
    );
    assert!(log.text().ends_with("line 65535\n"));

    let giant = "x".repeat(MAX_LOG_BYTES * 2);
    let bounded = BoundedLog::from_text(&giant);
    assert!(bounded.text().len() <= MAX_LOG_BYTES);
    assert!(bounded.truncated());
}

#[test]
fn notify_observers_on_state_changes() {
    let runner = FakeRunner::new(Behavior::Exit(0));
    let service = JobService::new(runner);
    let notifications = Rc::new(Cell::new(0));
    let counter = notifications.clone();
    let _observer = service.observe(Rc::new(move || counter.set(counter.get() + 1)));
    service
        .enqueue(request(
            handle(
                "a1",
                "One",
                ExecutionMode::WholeSelection,
                ErrorPolicy::Continue,
                "command",
            ),
            &["/tmp/a"],
        ))
        .expect("job queues");
    assert_eq!(notifications.get(), 1, "queueing notifies");
    let before = notifications.get();
    settle(&service);
    assert!(
        notifications.get() > before,
        "completion notifies the dashboard"
    );
}

#[test]
fn command_arguments_only_accept_absolute_paths() {
    let program = ActionProgram::Command {
        program: OsString::from("true"),
        arguments: vec![crate::model::ArgumentToken::Paths],
    };
    assert_eq!(
        expand_command_arguments(&program, &[PathBuf::from("/tmp/a")], Path::new("/tmp")),
        Ok(vec![OsString::from("/tmp/a")])
    );
    assert!(expand_command_arguments(&program, &[PathBuf::from("a")], Path::new("/tmp")).is_err());
    let script = ActionProgram::Script {
        interpreter: OsString::from("bash"),
        interpreter_arguments: Vec::new(),
        script: PathBuf::from("/tmp/actions/a1/run.sh"),
        family: crate::model::InterpreterFamily::Shell,
    };
    assert_eq!(
        expand_command_arguments(&script, &[PathBuf::from("/tmp/a")], Path::new("/tmp")),
        Ok(Vec::new())
    );
}
