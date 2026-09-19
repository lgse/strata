// SPDX-License-Identifier: MIT

use std::{ffi::OsString, path::PathBuf, rc::Rc, time::Duration};

use crate::model::{ActionDefinition, ExecutionMode};
use crate::services::jobs::JobProgress;
use crate::services::{
    ActionAvailability, ActionEventSink, ActionHandle, ActionProgram, ActionRunEvent,
    ActionRunRequest, ActionRunner, CancelHandle, InvocationSource, JobRequest, JobService,
    JobSnapshot, JobStatus, ScriptProgress,
};

use super::*;

struct InstantRunner;

impl ActionRunner for InstantRunner {
    fn run(&self, _: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
        sink(ActionRunEvent::Exited {
            code: Some(0),
            signal: None,
            log: String::new(),
        });
        Rc::new(|| {})
    }
}

struct PendingRunner;

impl ActionRunner for PendingRunner {
    fn run(&self, _: &ActionRunRequest, _: ActionEventSink) -> CancelHandle {
        Rc::new(|| {})
    }
}

fn handle(id: &str, name: &str, icon: Option<&str>, mode: ExecutionMode) -> Rc<ActionHandle> {
    let mode = match mode {
        ExecutionMode::PerItem => "per-item",
        ExecutionMode::WholeSelection => "whole-selection",
    };
    let icon = icon
        .map(|icon| format!("icon = \"{icon}\"\n"))
        .unwrap_or_default();
    let definition = ActionDefinition::parse(&format!(
        "schema_version = 1\nid = \"{id}\"\nname = \"{name}\"\n{icon}\n[when]\n\n[run]\nruntime = \"command\"\nprogram = \"true\"\nmode = \"{mode}\"\n"
    ))
    .expect("test definition is valid");
    Rc::new(ActionHandle {
        definition,
        directory: PathBuf::from("/tmp/actions").join(id),
        availability: ActionAvailability::Available(ActionProgram::Command {
            program: OsString::from("/bin/true"),
            arguments: Vec::new(),
        }),
    })
}

fn snapshot(status: JobStatus, progress: JobProgress, mode: ExecutionMode) -> JobSnapshot {
    JobSnapshot {
        id: JobId(1),
        action_name: "Convert images".to_owned(),
        icon: Some("image".to_owned()),
        mode,
        parent: PathBuf::from("/home/user/Pictures"),
        status,
        progress,
        log: String::new(),
        log_truncated: false,
        created: Vec::new(),
        message: None,
        elapsed: Duration::from_secs(65),
    }
}

#[test]
fn the_collapsed_label_separates_active_work_from_history() {
    let service = JobService::new(Rc::new(PendingRunner));
    assert_eq!(indicator_label(&service), "");
    for index in 0..3 {
        service
            .enqueue(JobRequest {
                action: handle(
                    &format!("a{index}"),
                    "Convert images",
                    None,
                    ExecutionMode::WholeSelection,
                ),
                inputs: vec![PathBuf::from("/tmp/a.png")],
                parent: PathBuf::from("/tmp"),
                source: InvocationSource::Selection,
            })
            .expect("job queues");
    }
    service.pump();
    assert_eq!(
        indicator_label(&service),
        "2 jobs running · 1 queued",
        "active work is described without a misleading combined percentage"
    );

    let finished = JobService::new(Rc::new(InstantRunner));
    finished
        .enqueue(JobRequest {
            action: handle("a1", "One", None, ExecutionMode::WholeSelection),
            inputs: vec![PathBuf::from("/tmp/a.png")],
            parent: PathBuf::from("/tmp"),
            source: InvocationSource::Selection,
        })
        .expect("job queues");
    for _ in 0..32 {
        finished.pump();
    }
    assert_eq!(indicator_label(&finished), "1 job finished");
}

#[test]
fn the_collapsed_label_reports_failures_without_calling_them_running() {
    struct FailingRunner;
    impl ActionRunner for FailingRunner {
        fn run(&self, _: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
            sink(ActionRunEvent::Exited {
                code: Some(1),
                signal: None,
                log: String::new(),
            });
            Rc::new(|| {})
        }
    }
    let service = JobService::new(Rc::new(FailingRunner));
    service
        .enqueue(JobRequest {
            action: handle("a1", "One", None, ExecutionMode::WholeSelection),
            inputs: vec![PathBuf::from("/tmp/a.png")],
            parent: PathBuf::from("/tmp"),
            source: InvocationSource::Selection,
        })
        .expect("job queues");
    for _ in 0..32 {
        service.pump();
    }
    assert_eq!(indicator_label(&service), "1 job finished · failures");
}

#[test]
fn status_labels_describe_partial_results_and_item_progress() {
    let per_item = JobProgress {
        completed_items: 42,
        succeeded_items: 42,
        total_items: 100,
        ..JobProgress::default()
    };
    assert_eq!(
        status_label(&snapshot(
            JobStatus::Running,
            per_item.clone(),
            ExecutionMode::PerItem
        )),
        "42 / 100"
    );

    let partial = JobProgress {
        completed_items: 20,
        succeeded_items: 18,
        failed_items: 2,
        total_items: 20,
        ..JobProgress::default()
    };
    assert_eq!(
        status_label(&snapshot(
            JobStatus::Failed,
            partial,
            ExecutionMode::PerItem
        )),
        "2 items failed"
    );
    assert_eq!(
        status_label(&snapshot(
            JobStatus::Succeeded,
            per_item.clone(),
            ExecutionMode::PerItem
        )),
        "Completed"
    );
    assert_eq!(
        status_label(&snapshot(
            JobStatus::Cancelled,
            per_item,
            ExecutionMode::PerItem
        )),
        "Cancelled"
    );
    assert_eq!(
        status_label(&snapshot(
            JobStatus::Running,
            JobProgress::default(),
            ExecutionMode::WholeSelection
        )),
        "Running",
        "an unmodified command shows no fabricated percentage"
    );
    assert_eq!(
        status_label(&snapshot(
            JobStatus::Queued,
            JobProgress::default(),
            ExecutionMode::WholeSelection
        )),
        "Queued"
    );
}

#[test]
fn meta_lines_show_progress_elapsed_time_and_failures() {
    let mut item = snapshot(
        JobStatus::Running,
        JobProgress {
            completed_items: 1,
            total_items: 4,
            message: Some("Converting images".to_owned()),
            ..JobProgress::default()
        },
        ExecutionMode::PerItem,
    );
    let label = meta_label(&item);
    assert!(label.contains("Converting images"), "{label}");
    assert!(label.contains("1m 05s elapsed"), "{label}");

    item.status = JobStatus::Failed;
    item.message = Some("Convert images exited with status 3".to_owned());
    let label = meta_label(&item);
    assert!(label.contains("exited with status 3"), "{label}");

    item.status = JobStatus::Succeeded;
    let label = meta_label(&item);
    assert!(
        !label.contains("exited with status 3"),
        "a successful run does not carry a stale failure message: {label}"
    );
}

#[test]
fn elapsed_time_is_readable_at_every_scale() {
    assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
    assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
    assert_eq!(format_elapsed(Duration::from_secs(65)), "1m 05s");
    assert_eq!(format_elapsed(Duration::from_secs(3661)), "1h 01m");
}

#[test]
fn job_icons_fall_back_to_a_bundled_asset() {
    let mut job = snapshot(
        JobStatus::Running,
        JobProgress::default(),
        ExecutionMode::WholeSelection,
    );
    assert_eq!(job_icon(&job), crate::assets::icons::PICTURES);
    job.icon = Some("not-a-bundled-icon".to_owned());
    assert_eq!(
        job_icon(&job),
        crate::ui::actions::DEFAULT_ACTION_ICON,
        "an unknown icon name never breaks the row"
    );
    job.icon = None;
    assert_eq!(job_icon(&job), crate::ui::actions::DEFAULT_ACTION_ICON);
}

fn descendants(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.as_ref().clone()];
    let mut child = widget.as_ref().first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        widgets.extend(descendants(&widget));
    }
    widgets
}

fn dashboard_button(root: &impl IsA<gtk::Widget>, label: &str) -> gtk::Button {
    descendants(root)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.tooltip_text().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("missing dashboard action: {label}"))
}

#[test]
fn finished_jobs_and_details_survive_the_indicator_builder() {
    crate::test_support::gtk_test(
        "ui::jobs::tests::finished_jobs_and_details_survive_the_indicator_builder",
        || {
            let service = JobService::new(Rc::new(InstantRunner));
            let indicator = JobsIndicator::with_service(service.clone());
            let widget = indicator.widget().clone();
            let window = gtk::Window::builder().child(&widget).build();
            // Window composition retains the widget, not the builder.
            drop(indicator);
            window.present();
            for index in 0..2 {
                service
                    .enqueue(JobRequest {
                        action: handle(
                            &format!("action{index}"),
                            "Finished action",
                            None,
                            ExecutionMode::WholeSelection,
                        ),
                        inputs: vec![PathBuf::from("/example/input")],
                        parent: PathBuf::from("/example"),
                        source: InvocationSource::Selection,
                    })
                    .expect("queue");
            }
            for _ in 0..4 {
                service.pump();
            }
            assert_eq!(service.finished_count(), 2);
            widget.popup();
            let popover = widget.popover().expect("dashboard");
            dashboard_button(&popover, "Details").emit_clicked();
            let details = descendants(&popover)
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Revealer>().ok())
                .any(|revealer| revealer.reveals_child());
            assert!(details, "finished output can be expanded");
            dashboard_button(&popover, "Hide").emit_clicked();
            assert!(
                descendants(&popover)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Revealer>().ok())
                    .all(|revealer| !revealer.reveals_child())
            );
            dashboard_button(&popover, "Minimize").emit_clicked();
            assert_eq!(service.finished_count(), 2, "minimizing retains history");
            widget.popup();
            dashboard_button(&popover, "Dismiss").emit_clicked();
            assert_eq!(service.finished_count(), 1);
            dashboard_button(&popover, "Details");
            service
                .enqueue(JobRequest {
                    action: handle(
                        "live-update",
                        "Another action",
                        None,
                        ExecutionMode::WholeSelection,
                    ),
                    inputs: vec![PathBuf::from("/example/input")],
                    parent: PathBuf::from("/example"),
                    source: InvocationSource::Selection,
                })
                .expect("queue while dashboard is open");
            dashboard_button(&popover, "Remove");
            service.pump();
            dashboard_button(&popover, "Cancel");
            service.pump();
            assert_eq!(service.finished_count(), 2);
            assert!(
                !descendants(&popover)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                    .any(|button| button.tooltip_text().as_deref() == Some("Cancel"))
            );
            dashboard_button(&popover, "Clear finished").emit_clicked();
            assert_eq!(service.finished_count(), 0);
            assert!(!widget.is_visible());
            window.close();
        },
    );
}

#[test]
fn the_last_window_cannot_abandon_queued_or_cancelling_jobs() {
    crate::test_support::gtk_test(
        "ui::jobs::tests::the_last_window_cannot_abandon_queued_or_cancelling_jobs",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.JobCloseTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(gio::Cancellable::NONE)
                .expect("register");
            let service = JobService::new(Rc::new(InstantRunner));
            let first = gtk::Window::builder().application(&application).build();
            let second = gtk::Window::builder().application(&application).build();
            let indicator = JobsIndicator::with_service(service.clone());
            let other = JobsIndicator::with_service(service.clone());
            first.set_child(Some(indicator.widget()));
            second.set_child(Some(other.widget()));
            indicator.bind_window(&first);
            other.bind_window(&second);
            let id = service
                .enqueue(JobRequest {
                    action: handle("close", "Close test", None, ExecutionMode::WholeSelection),
                    inputs: vec![PathBuf::from("/example/input")],
                    parent: PathBuf::from("/example"),
                    source: InvocationSource::Selection,
                })
                .expect("queue");
            assert!(!first.emit_by_name::<bool>("close-request", &[]));
            first.destroy();
            assert!(second.emit_by_name::<bool>("close-request", &[]));
            service.pump();
            service.cancel(id);
            assert!(second.emit_by_name::<bool>("close-request", &[]));
            service.pump();
            assert!(!second.emit_by_name::<bool>("close-request", &[]));
            second.destroy();
        },
    );
}

#[test]
fn new_jobs_open_in_the_launching_window_without_reopening_on_progress() {
    crate::test_support::gtk_test(
        "ui::jobs::tests::new_jobs_open_in_the_launching_window_without_reopening_on_progress",
        || {
            struct ControlledRunner(Rc<RefCell<Vec<ActionEventSink>>>);
            impl ActionRunner for ControlledRunner {
                fn run(&self, _: &ActionRunRequest, sink: ActionEventSink) -> CancelHandle {
                    self.0.borrow_mut().push(sink);
                    Rc::new(|| {})
                }
            }
            let sinks = Rc::new(RefCell::new(Vec::new()));
            let service = JobService::new(Rc::new(ControlledRunner(sinks.clone())));
            let indicator = JobsIndicator::with_service(service.clone());
            let other = JobsIndicator::with_service(service.clone());
            let anchor = gtk::Button::with_label("Launch action");
            let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
            body.append(&anchor);
            body.append(indicator.widget());
            let window = gtk::Window::builder().child(&body).build();
            let other_window = gtk::Window::builder().child(other.widget()).build();
            indicator.bind_window(&window);
            other.bind_window(&other_window);
            window.present();
            other_window.present();
            let enqueue = |id: &str| {
                service
                    .enqueue(JobRequest {
                        action: handle(id, id, None, ExecutionMode::WholeSelection),
                        inputs: vec![PathBuf::from("/example/input")],
                        parent: PathBuf::from("/example"),
                        source: InvocationSource::Selection,
                    })
                    .expect("queue")
            };
            let older = enqueue("older");
            service.pump();
            let newest = enqueue("newest");
            present_for(&anchor, newest);
            let main = glib::MainContext::default();
            while main.pending() {
                main.iteration(false);
            }
            let popover = indicator.widget().popover().expect("dashboard");
            assert!(popover.is_visible(), "launch opens its dashboard");
            assert!(
                !other
                    .widget()
                    .popover()
                    .expect("other dashboard")
                    .is_visible()
            );
            assert_eq!(dashboard_snapshots(&indicator.state)[0].id, newest);
            assert_eq!(
                service.snapshot()[0].id,
                older,
                "presentation does not reorder execution"
            );
            service.pump();
            let events = sinks.borrow().last().expect("new invocation").clone();
            events(ActionRunEvent::Progress(ScriptProgress {
                completed: 1,
                total: Some(4),
                message: Some("Renaming files".to_owned()),
            }));
            service.pump();
            assert!(
                descendants(&popover)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::ProgressBar>().ok())
                    .any(|bar| bar.fraction() == 0.25)
            );
            popover.popdown();
            events(ActionRunEvent::Progress(ScriptProgress {
                completed: 2,
                total: Some(4),
                message: None,
            }));
            service.pump();
            assert!(!popover.is_visible(), "updates respect Minimize");
            events(ActionRunEvent::Exited {
                code: Some(0),
                signal: None,
                log: "done".to_owned(),
            });
            service.pump();
            assert!(!popover.is_visible(), "completion respects Minimize");
            assert_eq!(dashboard_snapshots(&indicator.state)[0].id, newest);
            let next = enqueue("next");
            present_for(&anchor, next);
            while main.pending() {
                main.iteration(false);
            }
            assert!(popover.is_visible(), "a new launch opens it again");
            assert_eq!(dashboard_snapshots(&indicator.state)[0].id, next);
            window.close();
            other_window.close();
        },
    );
}
