use super::fs_quirks::{Births, HoldGuard, Op, Racer, Trash, hold};
use super::*;
use crate::adapters::local_operations::{
    Deadline, LocalDeleteJob, LocalDeleteQueue, PinnedEntry, RemovedReporter,
    parallel_delete_local_blocking_with_workers, parallel_delete_local_with,
    process_local_delete_job, retry_local_open, run_local_delete_workers,
};
use crate::{
    adapters::trash_support::SystemTrash,
    model::hidden_names::foreign_leftover,
    services::{Holder, file_activity},
};
use std::sync::{Barrier, Mutex, atomic::Ordering, mpsc};

#[test]
fn permanent_deletion_reports_injected_unlink_failures_without_removing_the_item()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for directory in [false, true] {
        let root = tempfile::tempdir()?;
        let path = root.path().join("item");
        if directory {
            fs::create_dir(&path)?;
        } else {
            fs::write(&path, b"keep")?;
        }
        let quirks = FsQuirks::install(
            root.path(),
            Quirks {
                fail: vec![(Op::Unlink, "item", 0, rustix::io::Errno::IO)],
                ..Quirks::default()
            },
        );
        let result = glib::MainContext::default().block_on(
            permanently_delete_local_path_if_unchanged(path.clone(), None, gio::Cancellable::new()),
        );
        assert!(result.is_err());
        assert!(path.exists());
        assert_eq!(quirks.count(Op::Unlink), 1);
        if !directory {
            assert_eq!(fs::read(&path)?, b"keep");
        }
    }
    Ok(())
}

fn delete_root(path: &Path) -> Result<PinnedEntry, Box<dyn Error>> {
    Ok(PinnedEntry {
        parent: Arc::new(super::super::open_local_parent_directory(
            path.parent().ok_or("no parent")?,
        )?),
        name: path.file_name().ok_or("no name")?.to_owned(),
        identity: None,
    })
}

fn enqueue_root(queue: &Arc<LocalDeleteQueue>, root: PinnedEntry) {
    queue.enqueue(LocalDeleteJob::Entry {
        parent: root.parent,
        name: root.name,
        expected: root.identity,
        guard: None,
        completion: None,
        deadline: Default::default(),
    });
}

fn process_next(queue: &Arc<LocalDeleteQueue>) {
    process_local_delete_job(queue, queue.next_job().expect("queued job"));
    queue.finish_job();
}

#[test]
fn permanent_delete_stops_if_an_open_directory_is_moved() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    let moved = root.path().join("moved");
    let sibling = root.path().join("sibling");
    fs::create_dir(&target)?;
    fs::write(target.join("child"), b"keep")?;
    fs::write(&sibling, b"keep sibling")?;
    let queue = Arc::new(LocalDeleteQueue::new(Arc::new(AtomicBool::new(false))));
    enqueue_root(&queue, delete_root(&sibling)?);
    enqueue_root(&queue, delete_root(&target)?);
    process_next(&queue);
    fs::rename(&target, &moved)?;
    fs::create_dir(&target)?;
    fs::write(target.join("replacement"), b"keep replacement")?;

    run_local_delete_workers(&queue, 1, |work| thread::Builder::new().spawn(work));

    let error = queue
        .result()
        .expect_err("moved directory must stop deletion");
    assert!(error.contains("changed"));
    assert_eq!(fs::read(moved.join("child"))?, b"keep");
    assert_eq!(fs::read(target.join("replacement"))?, b"keep replacement");
    assert_eq!(fs::read(sibling)?, b"keep sibling");
    Ok(())
}

#[test]
fn cancellation_after_enumeration_stops_and_joins_workers() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let target = root.path().join("target");
    fs::create_dir(&target)?;
    fs::write(target.join("first"), b"contents")?;
    fs::write(target.join("second"), b"contents")?;
    let cancellable = gio::Cancellable::new();
    let cancel = cancellable.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let barrier = Arc::new(Barrier::new(2));
    let cancel_barrier = barrier.clone();
    let canceller = thread::spawn(move || {
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker reached cancellation point");
        cancel.cancel();
        cancel_barrier.wait();
    });
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    let error = glib::MainContext::default()
        .block_on(parallel_delete_local_with(
            vec![delete_root(&target)?],
            cancellable,
            move |roots, cancelled| {
                let queue = Arc::new(LocalDeleteQueue::new(cancelled));
                for root in roots {
                    enqueue_root(&queue, root);
                }
                process_next(&queue);
                process_next(&queue);
                ready_tx.send(()).expect("canceller is waiting");
                barrier.wait();
                run_local_delete_workers(&queue, 2, |work| {
                    let finished = worker_finished.clone();
                    thread::Builder::new().spawn(move || {
                        work();
                        finished.store(true, Ordering::Release);
                    })
                });
                queue.result()
            },
        ))
        .expect_err("in-flight delete must be cancelled");
    canceller.join().expect("canceller finished");
    assert!(super::super::was_cancelled(&error));
    assert!(finished.load(Ordering::Acquire));
    assert_eq!(fs::read_dir(&target)?.count(), 1);
    assert!(!root.path().read_dir()?.any(|entry| {
        entry
            .expect("fixture entry")
            .file_name()
            .to_string_lossy()
            .starts_with(".strata-trash-")
    }));
    Ok(())
}

#[test]
fn a_worker_start_failure_stops_and_joins_started_workers() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("keep");
    fs::write(&target, b"keep")?;
    let queue = Arc::new(LocalDeleteQueue::new(Arc::new(AtomicBool::new(false))));
    enqueue_root(&queue, delete_root(&target)?);
    let finished = Arc::new(AtomicBool::new(false));
    let mut attempts = 0;
    run_local_delete_workers(&queue, 2, |work| {
        attempts += 1;
        if attempts == 2 {
            return Err(io::Error::other("injected spawn failure"));
        }
        let queue = queue.clone();
        let finished = finished.clone();
        thread::Builder::new().spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !queue.is_stopped() {
                assert!(Instant::now() < deadline, "queue was not stopped");
                thread::yield_now();
            }
            work();
            finished.store(true, Ordering::Release);
        })
    });
    assert!(
        queue
            .result()
            .expect_err("worker start must fail")
            .contains("injected spawn failure")
    );
    assert!(finished.load(Ordering::Acquire));
    assert_eq!(fs::read(target)?, b"keep");
    Ok(())
}

#[test]
fn parallel_workers_finish_nested_trees_without_following_symlinks() -> Result<(), Box<dyn Error>> {
    for workers in [1, 2, 4] {
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        let sentinel = root.path().join("sentinel");
        fs::write(&sentinel, b"keep")?;
        for index in 0..32 {
            let nested = target.join(format!("{index}/nested"));
            fs::create_dir_all(&nested)?;
            fs::write(nested.join("file"), b"delete")?;
            std::os::unix::fs::symlink(&sentinel, nested.join("link"))?;
        }
        parallel_delete_local_blocking_with_workers(
            vec![delete_root(&target)?],
            Arc::new(AtomicBool::new(false)),
            workers,
            &Default::default(),
        )?;
        assert!(!target.exists());
        assert_eq!(fs::read(sentinel)?, b"keep");
    }
    Ok(())
}

#[test]
fn workers_count_every_file_symlink_and_folder_they_remove() -> Result<(), Box<dyn Error>> {
    for workers in [1, 2, 4] {
        let root = tempfile::tempdir()?;
        let target = root.path().join("target");
        for index in 0..8 {
            let nested = target.join(format!("{index}/nested"));
            fs::create_dir_all(&nested)?;
            fs::write(nested.join("file"), b"delete")?;
            std::os::unix::fs::symlink("file", nested.join("link"))?;
        }
        let deadline = Deadline::default();
        parallel_delete_local_blocking_with_workers(
            vec![delete_root(&target)?],
            Arc::new(AtomicBool::new(false)),
            workers,
            &deadline,
        )?;
        // The target, then per index its folder, its nested folder, a file and a symlink.
        assert_eq!(deadline.removed(), 1 + 8 * 4, "{workers} workers");
    }
    Ok(())
}

#[test]
fn removed_entries_are_reported_when_they_change_and_stop_with_the_reporter() {
    let _serial = serial().expect("serialized");
    let events = Rc::new(RefCell::new(Vec::new()));
    let deadline = Deadline::default();
    let recorded = events.clone();
    let reporter = RemovedReporter::start(
        OperationRequestId(7),
        deadline.clone(),
        Rc::new(move |event| recorded.borrow_mut().push(event)),
        Duration::from_millis(5),
    );
    let context = glib::MainContext::default();
    let wait = |ms: u64| context.block_on(glib::timeout_future(Duration::from_millis(ms)));
    let removed = || {
        events
            .borrow()
            .iter()
            .map(|event| match event {
                OperationEvent::DeleteEntriesRemoved {
                    request_id,
                    removed,
                } => (request_id.0, *removed),
                other => panic!("unexpected event {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    let wait_for_reports = |count: usize| {
        let started = std::time::Instant::now();
        while removed().len() < count && started.elapsed() < Duration::from_secs(5) {
            wait(5);
        }
    };

    wait(40);
    assert!(removed().is_empty(), "nothing removed yet");
    (0..3).for_each(|_| deadline.note_removed());
    wait_for_reports(1);
    wait(40);
    assert_eq!(removed(), [(7, 3)], "an unchanged count is reported once");
    deadline.note_removed();
    wait_for_reports(2);
    assert_eq!(removed(), [(7, 3), (7, 4)]);

    drop(reporter);
    deadline.note_removed();
    wait(40);
    assert_eq!(removed().len(), 2, "a dropped reporter stays silent");
}

#[test]
fn transient_open_failures_retry_but_cannot_loop_forever() -> Result<(), Box<dyn Error>> {
    use rustix::io::Errno;
    for error in [Errno::AGAIN, Errno::INTR, Errno::ACCESS] {
        let mut attempts = 0;
        let result = retry_local_open(|| {
            attempts += 1;
            assert!(attempts <= 32, "unbounded retry");
            Err(error)
        });
        assert_eq!(result.expect_err("injected open failure"), error);
        if error == Errno::ACCESS {
            assert_eq!(attempts, 1);
        } else {
            assert!(attempts > 1);
        }
    }
    let mut attempts = 0;
    let fd = retry_local_open(|| {
        attempts += 1;
        match attempts {
            1 => Err(Errno::AGAIN),
            2 => Err(Errno::INTR),
            _ => rustix::fs::open(
                ".",
                rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
                rustix::fs::Mode::empty(),
            ),
        }
    })?;
    assert_eq!(attempts, 3);
    assert!(rustix::fs::fstat(fd).is_ok());
    Ok(())
}

#[test]
fn deletion_error_summaries_are_bounded_and_report_the_failure_count() {
    let errors = (1..=10)
        .map(|index| format!("item-{index}: denied"))
        .collect::<Vec<_>>();

    let summary = deletion_error_summary(&errors, true);

    assert!(summary.starts_with("10 items could not be deleted"));
    assert!(summary.contains("• item-1: denied"));
    assert!(summary.contains("• item-8: denied"));
    assert!(!summary.contains("• item-9: denied"));
    assert!(summary.ends_with("… and 2 more items"));
    assert!(
        deletion_error_summary(&errors[..1], false)
            .starts_with("1 item could not be moved to Trash")
    );
    assert!(
        operation_error_summary(&errors[..1], FailedAction::Restore)
            .starts_with("1 item could not be restored")
    );
}

#[test]
fn large_deletion_progress_reports_every_completed_location() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let mut entries = Vec::new();
    let mut expected = HashSet::new();
    for index in 0..40 {
        let path = root.path().join(format!("file-{index:02}.txt"));
        fs::write(&path, index.to_string())?;
        expected.insert(Location::local(&path));
        entries.push(file_entry(&path));
    }

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(24),
            entries,
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        glib::MainContext::default().iteration(true);
    }

    let reported = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            OperationEvent::DeleteProgress {
                deleted_locations, ..
            } => Some(deleted_locations),
            _ => None,
        })
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    assert_eq!(reported, expected);
    Ok(())
}

#[test]
fn deletion_failures_explain_trash_problems_and_keep_other_details() {
    use gio::IOErrorEnum as Io;
    let unsupported = TrashError::from(glib::Error::new(Io::NotSupported, "trash not supported"));
    let cases = [
        (
            unsupported,
            true,
            "share-folder: This location doesn't support Trash.",
        ),
        (
            TrashError::permanent(glib::Error::new(Io::NotSupported, "trash not supported")),
            false,
            "share-folder: trash not supported",
        ),
        (
            TrashError::from(glib::Error::new(Io::ReadOnly, "read-only")),
            false,
            "share-folder: This location is read-only.",
        ),
        (
            TrashError::from(glib::Error::new(Io::NoSpace, "full")),
            false,
            "share-folder: Not enough space to move to Trash.",
        ),
        (
            TrashError::from(glib::Error::new(
                Io::PermissionDenied,
                "Unable to trash file /tmp/secret.txt: access denied",
            )),
            false,
            "share-folder: Permission denied",
        ),
        (
            TrashError::from(glib::Error::new(Io::Failed, "disk exploded")),
            false,
            "share-folder: disk exploded",
        ),
    ];
    for (error, retryable, expected) in cases {
        assert_eq!(error.is_unsupported(), retryable, "{expected}");
        assert_eq!(deletion_error_message("share-folder", &error), expected);
    }
}

#[test]
fn a_refused_trash_is_retryable_and_remembered_only_on_fuse() -> Result<(), Box<dyn Error>> {
    use crate::adapters::trash_support::{
        tests::RefusingMount, tests::leftover_trash, unsupported_reason_in,
    };
    let _serial = serial()?;
    for (fs_type, unsupported) in [("fuse.sshfs", true), ("ext4", false)] {
        let root = tempfile::tempdir()?;
        let doomed = root.path().join("doomed.txt");
        fs::write(&doomed, b"doomed")?;
        let environment = Rc::new(RefusingMount::at(root.path(), 7301, fs_type));
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();
        glib::MainContext::default().block_on(run_deletion_in(
            environment.clone(),
            OperationRequestId(73),
            vec![DeletionTarget::from_entry(&file_entry(&doomed))],
            false,
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            gio::Cancellable::new(),
        ));

        let events = events.borrow();
        let Some(OperationEvent::CompletedWithErrors {
            retryable_locations,
            has_non_retryable_failures,
            message,
            ..
        }) = events.last()
        else {
            panic!("{fs_type}: expected a partial failure, got {events:?}");
        };
        let expected_retryable = if unsupported {
            vec![Location::local(&doomed)]
        } else {
            Vec::new()
        };
        assert_eq!(retryable_locations, &expected_retryable, "{fs_type}");
        assert_eq!(*has_non_retryable_failures, !unsupported, "{fs_type}");
        assert_eq!(
            message.contains("doomed.txt: This location doesn't support Trash."),
            unsupported,
            "{fs_type}: {message}"
        );
        assert_eq!(
            unsupported_reason_in(&environment.mounts, &doomed).is_some(),
            unsupported,
            "{fs_type}"
        );
        assert_eq!(
            leftover_trash(root.path()).exists(),
            !unsupported,
            "{fs_type}"
        );
        assert!(doomed.exists(), "{fs_type}");
    }
    Ok(())
}

#[test]
fn provider_cleans_only_confirmed_pins_on_failure_and_cancellation() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::provider_cleans_only_confirmed_pins_on_failure_and_cancellation",
        || {
            for cancel_after_first in [false, true] {
                let directory = tempfile::tempdir().expect("fixture");
                let first = directory.path().join("first");
                let second = directory.path().join("second");
                fs::create_dir(&first).expect("first folder");
                if cancel_after_first {
                    fs::create_dir(&second).expect("second folder");
                }
                let bookmarks = crate::adapters::bookmarks::pinned_places_path();
                fs::create_dir_all(bookmarks.parent().expect("bookmarks directory"))
                    .expect("config");
                let pin = |path: &Path| format!("{} Pin\n", gio::File::for_path(path).uri());
                fs::write(&bookmarks, format!("{}{}", pin(&first), pin(&second))).expect("pins");
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let operation = Rc::new(RefCell::new(None::<LoadHandle>));
                let cancelling = operation.clone();
                operation.replace(Some(LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(1372),
                        entries: vec![directory_entry(&first), directory_entry(&second)],
                        permanent: true,
                    },
                    Rc::new(move |event| {
                        if cancel_after_first
                            && matches!(event, OperationEvent::DeleteProgress { completed: 1, .. })
                        {
                            cancelling.borrow_mut().take();
                        }
                        emitted.borrow_mut().push(event);
                    }),
                )));
                let context = glib::MainContext::default();
                while !events.borrow().iter().any(|event| {
                    matches!(
                        event,
                        OperationEvent::Cancelled { .. }
                            | OperationEvent::CompletedWithErrors { .. }
                    )
                }) {
                    context.iteration(true);
                }
                crate::storage::writer::settle();
                assert_eq!(
                    fs::read_to_string(&bookmarks).expect("remaining pins"),
                    pin(&second)
                );
                let events = events.borrow();
                if cancel_after_first {
                    assert!(events.iter().any(|event| matches!(event, OperationEvent::Cancelled { result, .. } if result.completed == [Location::local(&first)] && result.not_attempted == [Location::local(&second)])));
                    assert!(second.exists());
                } else {
                    assert!(events.iter().any(|event| matches!(event, OperationEvent::CompletedWithErrors { deleted_locations, .. } if deleted_locations == &[Location::local(&first)])));
                }
                assert!(!first.exists());
            }
        },
    );
}

#[test]
fn cancelling_between_deletions_reports_completed_and_unattempted_items()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-delete-cancel-test-{unique}"));
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    fs::create_dir_all(&root)?;
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let operation = Rc::new(RefCell::new(None::<LoadHandle>));
    let emitted = events.clone();
    let operation_for_emit = operation.clone();
    let handle = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(7),
            entries: vec![file_entry(&first), file_entry(&second)],
            permanent: true,
        },
        Rc::new(move |event| {
            let cancel = matches!(event, OperationEvent::DeleteProgress { completed: 1, .. });
            emitted.borrow_mut().push(event);
            if cancel {
                operation_for_emit.borrow_mut().take();
            }
        }),
    );
    operation.replace(Some(handle));
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Cancelled { .. }))
    {
        glib::MainContext::default().iteration(true);
    }

    let result = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            OperationEvent::Cancelled { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("terminal cancellation result");
    assert_eq!(result.completed, [Location::local(&first)]);
    assert!(result.failed.is_empty());
    assert_eq!(result.not_attempted, [Location::local(&second)]);
    assert!(!first.exists());
    assert!(second.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_cancelled_delete_counts_only_what_it_left_whole_as_not_attempted() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    for (row, permanent, cut_is_folder) in [
        ("to Trash", false, false),
        ("a file permanently", true, false),
        ("a folder permanently", true, true),
    ] {
        let root = tempfile::tempdir()?;
        let [denied, cut, rest] =
            ["denied.txt", "cut", "rest.txt"].map(|name| root.path().join(name));
        let (cut_entry, cut_inside) = if cut_is_folder {
            fs::create_dir(&cut)?;
            (directory_entry(&cut), cut.join("inside.txt"))
        } else {
            (file_entry(&cut), cut.clone())
        };
        for item in [&denied, &cut_inside, &rest] {
            fs::write(item, b"kept")?;
        }
        let cancellable = gio::Cancellable::new();
        let cancel = cancellable.clone();
        let cancel: super::fs_quirks::Racer = Box::new(move || cancel.cancel());
        let cancel_at = if !permanent {
            Quirks {
                before: vec![(Op::Trash, "cut", 0, cancel)],
                ..Quirks::default()
            }
        } else {
            let checked = if cut_is_folder { "inside.txt" } else { "cut" };
            Quirks {
                after: vec![(Op::Stat, checked, 0, cancel)],
                ..Quirks::default()
            }
        };
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                fail: vec![(Op::Stat, "denied.txt", 0, rustix::io::Errno::ACCESS)],
                ..cancel_at
            },
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let emitted = events.clone();

        glib::MainContext::default().block_on(super::super::run_deletion(
            OperationRequestId(1615),
            [file_entry(&denied), cut_entry, file_entry(&rest)]
                .iter()
                .map(super::super::DeletionTarget::from_entry)
                .collect(),
            permanent,
            Rc::new(move |event| emitted.borrow_mut().push(event)),
            cancellable,
        ));

        let events = events.borrow();
        let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
            panic!("{row}: expected a cancel, got {events:?}");
        };
        assert!(result.completed.is_empty(), "{row}");
        if permanent {
            assert!(
                !cut_inside.exists(),
                "{row}: the cancel landed before the delete"
            );
            assert_eq!(
                result.failed,
                [Location::local(&denied), Location::local(&cut)],
                "{row}"
            );
            assert_eq!(result.not_attempted, [Location::local(&rest)], "{row}");
        } else {
            assert_eq!(fs::read(&cut_inside)?, b"kept", "{row}");
            assert_eq!(result.failed, [Location::local(&denied)], "{row}");
            assert_eq!(
                result.not_attempted,
                [Location::local(&cut), Location::local(&rest)],
                "{row}"
            );
        }
        for item in [&denied, &rest] {
            assert_eq!(fs::read(item)?, b"kept", "{row}");
        }
    }
    Ok(())
}

#[test]
fn deleting_browser_can_drop_after_a_confirmed_deletion_without_losing_pin_cleanup() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::deleting_browser_can_drop_after_a_confirmed_deletion_without_losing_pin_cleanup",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let first = directory.path().join("first");
            let second = directory.path().join("second");
            fs::create_dir(&first).expect("first folder");
            fs::create_dir(&second).expect("second folder");
            let bookmarks = crate::adapters::bookmarks::pinned_places_path();
            fs::create_dir_all(bookmarks.parent().expect("bookmarks directory")).expect("config");
            let pin = |path: &Path| format!("{} Pin\n", gio::File::for_path(path).uri());
            fs::write(&bookmarks, format!("{}{}", pin(&first), pin(&second))).expect("pins");
            let browser = crate::app::Browser::new(Rc::new(crate::adapters::LocalFileSource));
            browser.set_operation_provider(Rc::new(LocalOperationProvider));
            let weak = Rc::downgrade(&browser);
            let owner = Rc::new(RefCell::new(Some(browser.clone())));
            let release = owner.clone();
            browser.observe(move |event| {
                if matches!(
                    event,
                    crate::app::BrowserEvent::DeletionProgress { completed: 1, .. }
                ) {
                    release
                        .borrow_mut()
                        .take()
                        .expect("origin browser")
                        .clear_observer();
                }
            });
            browser.delete(
                vec![directory_entry(&first), directory_entry(&second)],
                true,
            );
            drop(browser);
            let context = glib::MainContext::default();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while weak.upgrade().is_some() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "origin browser was retained"
                );
                context.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!first.exists());
            assert!(second.exists());
            crate::storage::writer::settle();
            assert_eq!(
                fs::read_to_string(bookmarks).expect("saved pins"),
                pin(&second)
            );
        },
    );
}

#[test]
fn bookmark_cleanup_failure_preserves_the_successful_deletion_outcome() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::bookmark_cleanup_failure_preserves_the_successful_deletion_outcome",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let gone = directory.path().join("gone");
            fs::create_dir(&gone).expect("folder");
            let bookmarks = crate::adapters::bookmarks::pinned_places_path();
            fs::create_dir_all(&bookmarks).expect("unreadable bookmark file");
            let failures = Rc::new(RefCell::new(Vec::new()));
            let observed = failures.clone();
            let _watch = crate::adapters::bookmarks::watch_changes(move |result| {
                observed.borrow_mut().push(result.is_err());
                true
            });
            let events = Rc::new(RefCell::new(Vec::new()));
            let emitted = events.clone();
            let _operation = LocalOperationProvider.delete(
                DeleteRequest {
                    id: OperationRequestId(1372),
                    entries: vec![directory_entry(&gone)],
                    permanent: true,
                },
                Rc::new(move |event| emitted.borrow_mut().push(event)),
            );
            while !events
                .borrow()
                .iter()
                .any(|event| matches!(event, OperationEvent::Deleted { .. }))
            {
                glib::MainContext::default().iteration(true);
            }
            crate::storage::writer::settle();
            assert!(!gone.exists());
            assert_eq!(*failures.borrow(), [true]);
            assert!(
                matches!(events.borrow().last(), Some(OperationEvent::Deleted { locations, .. }) if locations == &[Location::local(&gone)])
            );
            assert!(!events.borrow().iter().any(|event| matches!(
                event,
                OperationEvent::Failed { .. } | OperationEvent::CompletedWithErrors { .. }
            )));
        },
    );
}

#[test]
fn permanent_delete_removes_a_symlink_standing_in_for_a_directory_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let outside = std::env::temp_dir().join(format!("strata-delete-symlink-outside-{unique}"));
    let decoy = std::env::temp_dir().join(format!("strata-delete-symlink-decoy-{unique}"));
    fs::create_dir_all(&outside)?;
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"do not delete me")?;
    // `directory_entry` reports `kind: Directory` even though the entry is
    // actually a symlink on disk, standing in for a `FileEntry` whose type
    // went stale because the real directory was swapped for a symlink.
    std::os::unix::fs::symlink(&outside, &decoy)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(20),
            entries: vec![directory_entry(&decoy)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        context.iteration(true);
    }

    assert!(
        sentinel.exists(),
        "deleting the symlink must never touch what it points to"
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 1);
    assert!(!decoy.exists() && !decoy.is_symlink());

    fs::remove_dir_all(outside)?;
    Ok(())
}

#[test]
fn permanent_delete_does_not_follow_a_symlink_nested_inside_the_tree() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-delete-nested-symlink-test-{unique}"));
    let outside = std::env::temp_dir().join(format!("strata-delete-nested-outside-{unique}"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;
    fs::create_dir_all(&outside)?;
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"do not delete me")?;
    fs::write(nested.join("visible.txt"), b"contents")?;
    std::os::unix::fs::symlink(&outside, nested.join("decoy"))?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(21),
            entries: vec![directory_entry(&root)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        context.iteration(true);
    }

    assert!(
        sentinel.exists(),
        "a symlink nested inside the deleted tree must never lead outside it"
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 1);
    assert!(!root.exists());

    fs::remove_dir_all(outside)?;
    Ok(())
}

#[test]
fn permanent_delete_accepts_a_symlink_in_the_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    let target = actual_parent.join("target.txt");
    fs::create_dir(&actual_parent)?;
    fs::write(&target, b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(22),
            entries: vec![file_entry(&linked_parent.join("target.txt"))],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Deleted { .. }
                | OperationEvent::CompletedWithErrors { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        context.iteration(true);
    }

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Deleted { .. })),
        "{:?}",
        events.borrow()
    );
    assert!(!target.exists());
    assert!(linked_parent.is_symlink());
    assert!(actual_parent.is_dir());
    Ok(())
}

#[test]
fn permanent_delete_keeps_the_open_parent_when_its_alias_is_retargeted()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let outside = root.path().join("outside");
    let alias = root.path().join("alias");
    fs::create_dir_all(actual.join("tree/nested"))?;
    fs::create_dir_all(outside.join("tree"))?;
    fs::write(actual.join("tree/nested/file.txt"), b"delete")?;
    fs::write(outside.join("tree/sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink(&outside, actual.join("tree/decoy"))?;
    std::os::unix::fs::symlink(&actual, &alias)?;
    let parent = super::open_local_parent_directory(&alias)?;

    std::os::unix::fs::symlink(&outside, root.path().join("replacement"))?;
    fs::rename(root.path().join("replacement"), &alias)?;
    glib::MainContext::default().block_on(super::permanently_delete_local(
        parent,
        OsString::from("tree"),
        None,
        gio::Cancellable::new(),
    ))?;

    assert!(!actual.join("tree").exists());
    assert_eq!(fs::read(outside.join("tree/sentinel.txt"))?, b"keep");
    assert_eq!(fs::read_link(alias)?, outside);
    Ok(())
}

#[test]
fn permanent_delete_revalidates_identity_after_a_parent_alias_changes() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let outside = root.path().join("outside");
    let alias = root.path().join("alias");
    fs::create_dir(&actual)?;
    fs::create_dir(&outside)?;
    fs::write(actual.join("file.txt"), b"original")?;
    fs::write(outside.join("file.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual, &alias)?;
    let file = gio::File::for_path(alias.join("file.txt"));
    let context = glib::MainContext::default();
    let expected = context.block_on(super::local_file_identity(&file))?;

    std::os::unix::fs::symlink(&outside, root.path().join("replacement"))?;
    fs::rename(root.path().join("replacement"), &alias)?;
    let error = context
        .block_on(super::permanently_delete_local_path_if_unchanged(
            alias.join("file.txt"),
            expected,
            gio::Cancellable::new(),
        ))
        .expect_err("retargeting the alias must not delete a different entry");

    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(fs::read(actual.join("file.txt"))?, b"original");
    assert_eq!(fs::read(outside.join("file.txt"))?, b"keep");
    Ok(())
}

#[test]
fn a_removal_by_a_recorded_identity_follows_the_mounts_rule() -> Result<(), Box<dyn Error>> {
    #[derive(Clone, Copy, Debug)]
    enum Since {
        Evicted,
        Replaced,
        ReplacedAlike,
        ReplacedOnceNumbersWent,
    }
    let _serial = serial()?;
    let samba = || Quirks {
        births: Births::MadeUp,
        ..Quirks::cifs()
    };
    let rows: [(Preset, Since, bool); 5] = [
        (Quirks::vfat, Since::Evicted, true),
        (Quirks::vfat, Since::Replaced, false),
        (Quirks::ext4, Since::Evicted, false),
        (samba, Since::ReplacedAlike, false),
        (samba, Since::ReplacedOnceNumbersWent, false),
    ];
    for (preset, since, removed) in rows {
        for permanently in [true, false] {
            let row = format!("{:?}, {since:?}, {permanently}", preset().mount_type);
            let root = tempfile::tempdir()?;
            let path = root.path().join("item");
            let theirs = root.path().join("theirs");
            fs::write(&path, b"recorded")?;
            fs::write(&theirs, b"theirs")?;
            let hour_before = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)?
                .as_secs()
                - 3600;
            for file in [&path, &theirs] {
                set_times(file, hour_before, 0)?;
            }
            let _scope = FsQuirks::install(root.path(), preset());
            let context = glib::MainContext::default();
            let expected = context
                .block_on(super::local_file_identity(&gio::File::for_path(&path)))?
                .ok_or("a local item")?;
            match since {
                Since::Evicted => assert!(super::fs_quirks::evict(&path), "{row}"),
                Since::Replaced => {
                    let probe = root.path().join("probe");
                    wait_for_a_later_birth_time(&probe, &fs::metadata(&path)?);
                    fs::remove_file(&path)?;
                    fs::write(&path, b"theirs")?;
                    super::fs_quirks::recreated(&path);
                }
                Since::ReplacedAlike => fs::rename(&theirs, &path)?,
                Since::ReplacedOnceNumbersWent => {
                    assert!(super::fs_quirks::autodisable_serverino(&path), "{row}");
                    fs::rename(&theirs, &path)?;
                }
            }

            let gone = if permanently {
                let result = context.block_on(super::permanently_delete_local_path_if_unchanged(
                    path.clone(),
                    Some(expected),
                    gio::Cancellable::new(),
                ));
                if let Err(error) = &result {
                    assert!(error.to_string().contains("changed"), "{row}: {error}");
                }
                result.is_ok()
            } else {
                let parent = super::open_local_parent_directory(root.path())?;
                let item = std::ffi::OsStr::new("item");
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            super::super::placement::remove_if_unchanged(
                                super::super::fs_ops::fs(),
                                super::super::fs_ops::Entry::new(&parent, item),
                                expected.identity,
                                expected.attributes,
                                rustix::fs::AtFlags::empty(),
                            )
                        })
                        .join()
                })
                .map_err(|_| "the removal panicked")?
            };

            assert_eq!(gone, removed, "{row}");
            assert_eq!(path.exists(), !removed, "{row}");
        }
    }
    Ok(())
}

#[test]
fn permanent_delete_of_a_symlink_through_an_alias_keeps_its_referent() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    fs::create_dir(&actual)?;
    fs::write(actual.join("sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    std::os::unix::fs::symlink(".", actual.join("link"))?;

    glib::MainContext::default().block_on(super::permanently_delete_local_path_if_unchanged(
        alias.join("link"),
        None,
        gio::Cancellable::new(),
    ))?;

    assert!(!actual.join("link").is_symlink());
    assert_eq!(fs::read(actual.join("sentinel.txt"))?, b"keep");
    assert!(alias.is_symlink());
    Ok(())
}

#[test]
fn an_already_cancelled_recursive_delete_preserves_the_root() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-recursive-delete-cancel-test-{unique}"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;
    for index in 0..64 {
        fs::write(nested.join(format!("item-{index}.txt")), b"contents")?;
    }

    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    let parent = super::open_local_parent_directory(root.parent().ok_or("no parent")?)?;
    let delete_root = super::PinnedEntry {
        parent: Arc::new(parent),
        name: root.file_name().ok_or("no name")?.to_owned(),
        identity: None,
    };
    let context = glib::MainContext::default();
    let error = context
        .block_on(super::parallel_delete_local(vec![delete_root], cancellable))
        .expect_err("cancelled delete must return error");

    assert!(super::was_cancelled(&error));
    assert!(root.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn trashing_and_restoring_report_each_items_identity_and_a_permanent_delete_reports_none() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::trashing_and_restoring_report_each_items_identity_and_a_permanent_delete_reports_none",
        || {
            for permanent in [false, true] {
                let root = tempfile::tempdir().expect("root");
                let folder = root.path().join("folder");
                fs::create_dir(&folder).expect("folder");
                let identity = numbers_at(&folder);
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let _operation = LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(95),
                        entries: vec![directory_entry(&folder)],
                        permanent,
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
                    )
                });

                let trashed = {
                    let events = events.borrow();
                    assert!(matches!(
                        events.last(),
                        Some(OperationEvent::Deleted { .. })
                    ));
                    assert_eq!(
                        trashed_items(&events),
                        if permanent {
                            Vec::new()
                        } else {
                            vec![(Location::local(&folder), identity)]
                        }
                    );
                    events.iter().find_map(|event| match event {
                        OperationEvent::ItemTrashed { identity, .. } => Some(*identity),
                        _ => None,
                    })
                };
                let Some(trashed) = trashed else {
                    continue;
                };
                let restoring = Rc::new(RefCell::new(Vec::new()));
                let emitted = restoring.clone();
                let _operation = LocalOperationProvider.restore(
                    RestoreRequest {
                        id: OperationRequestId(96),
                        source: RestoreSource::OriginalLocations {
                            locations: vec![Location::local(&folder)],
                            originals: HashMap::new(),
                        },
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&restoring, |event| {
                    matches!(
                        event,
                        OperationEvent::Restored { .. }
                            | OperationEvent::RestoreCompletedWithErrors { .. }
                            | OperationEvent::Failed { .. }
                    )
                });
                let restoring = restoring.borrow();
                let restored = restoring
                    .iter()
                    .filter_map(|event| match event {
                        OperationEvent::ItemRestored {
                            location, identity, ..
                        } => Some((location.clone(), *identity)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert!(
                    matches!(&restored[..], [(location, restored)]
                        if *location == Location::local(&folder) && restored.unchanged(&trashed)),
                    "{restored:?} after {:?}",
                    restoring.last()
                );
            }
        },
    );
}

#[derive(Clone, Copy, Debug)]
enum Swap {
    Untouched,
    RenamedOver,
    MovedAside,
}

#[test]
fn trash_reports_the_item_it_moved_when_another_took_the_name_first() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::trash_reports_the_item_it_moved_when_another_took_the_name_first",
        || {
            let rows: [(Preset, Op, Swap, bool); 5] = [
                (Quirks::ext4, Op::Trash, Swap::Untouched, false),
                (Quirks::ext4, Op::Trash, Swap::RenamedOver, true),
                (Quirks::ext4, Op::OpenAt, Swap::RenamedOver, true),
                (Quirks::sshfs, Op::Trash, Swap::MovedAside, true),
                (Quirks::nfs3, Op::Trash, Swap::RenamedOver, false),
            ];
            for (preset, op, swap, reports_arrival) in rows {
                let root = tempfile::tempdir().expect("root");
                let item = root.path().join("report.txt");
                let arrival = root.path().join("arrival.txt");
                fs::write(&item, b"checked").expect("item");
                fs::write(&arrival, b"arrived later").expect("arrival");
                let (checked, arrived) = (numbers_at(&item), numbers_at(&arrival));
                let (from, to, aside) = (
                    arrival.clone(),
                    item.clone(),
                    root.path().join("report.txt~"),
                );
                let racer: Racer = Box::new(move || match swap {
                    Swap::Untouched => {}
                    Swap::RenamedOver => fs::rename(&from, &to).expect("renamed over"),
                    Swap::MovedAside => {
                        fs::rename(&to, &aside).expect("moved aside");
                        fs::rename(&from, &to).expect("renamed in");
                    }
                });
                let _quirks = FsQuirks::install(
                    root.path(),
                    Quirks {
                        before: vec![(op, "report.txt", 0, racer)],
                        ..preset()
                    },
                );
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let _operation = LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(13),
                        entries: vec![file_entry(&item)],
                        permanent: false,
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
                    )
                });

                let events = events.borrow();
                assert!(
                    matches!(events.last(), Some(OperationEvent::Deleted { .. })),
                    "{op:?}, {swap:?}: {:?}",
                    events.last()
                );
                assert_eq!(
                    trashed_items(&events),
                    [(
                        Location::local(&item),
                        if reports_arrival { arrived } else { checked }
                    )],
                    "{:?}, {op:?}, {swap:?}",
                    preset().mount_type
                );
            }
        },
    );
}

#[test]
fn verified_trash_learns_fuse_refusals_and_cleans_leftovers_on_both_worker_routes() {
    const NAME: &str = "adapters::local_operations::tests::deletion::verified_trash_learns_fuse_refusals_and_cleans_leftovers_on_both_worker_routes";
    crate::test_support::gtk_test(NAME, || {
        use crate::adapters::trash_support::{self, TrashFailure, UnsupportedReason};
        let Some((_home, volume)) = crate::test_support::distinct_device_dirs(NAME) else {
            return;
        };
        for synchronous in [false, true] {
            for trash in [
                Trash::FalseWithoutError,
                Trash::SucceedsWithoutMoving,
                Trash::ProtocolRefused,
            ] {
                let root = tempfile::TempDir::new_in(volume.path()).expect("root");
                let folder = root.path().join("folder");
                fs::create_dir(&folder).expect("folder");
                fs::write(folder.join("keep"), b"keep").expect("contents");
                let leftovers = crate::adapters::trash_support::tests::leftover_trash(root.path());
                let created = leftovers.clone();
                let _quirks = FsQuirks::install(
                    root.path(),
                    Quirks {
                        trash,
                        before: vec![(
                            Op::Trash,
                            "folder",
                            0,
                            Box::new(move || {
                                fs::create_dir_all(created.join("files")).expect("files");
                                fs::create_dir_all(created.join("info")).expect("info");
                            }),
                        )],
                        ..Quirks::default()
                    },
                );
                crate::adapters::volume::set_mountinfo_for_isolated_test(Some(format!(
                    "9001 1 0:91 / {} rw - fuse.sshfs fixture rw\n",
                    root.path().display()
                )));
                let context = glib::MainContext::default();
                let result = if synchronous {
                    let path = folder.clone();
                    context
                        .block_on(super::super::run_blocking(move || {
                            let parent =
                                open_local_parent_directory(path.parent().expect("parent"))
                                    .expect("open");
                            let at =
                                super::super::Entry::new(&parent, path.file_name().expect("name"));
                            let stat = super::super::fs().stat(at, false).expect("stat");
                            Ok(super::super::trash::trash_verified(
                                at,
                                &path,
                                &stat,
                                &gio::Cancellable::new(),
                            )
                            .map(drop))
                        }))
                        .expect("worker")
                } else {
                    context
                        .block_on(trash_support::trash_with_cleanup(
                            &gio::File::for_path(&folder),
                            &gio::Cancellable::new(),
                        ))
                        .map(drop)
                };
                let expected = if matches!(trash, Trash::ProtocolRefused) {
                    UnsupportedReason::FuseRefused
                } else {
                    UnsupportedReason::Reported
                };
                assert_eq!(
                    result.expect_err("not moved").failure,
                    TrashFailure::Unsupported(expected)
                );
                assert_eq!(
                    trash_support::unsupported_reason_for(&folder).map(|(_, reason)| reason),
                    Some(expected)
                );
                assert!(!leftovers.exists(), "{synchronous:?} {trash:?}");
                assert_eq!(fs::read(folder.join("keep")).expect("kept"), b"keep");
                trash_support::forget_all();
                crate::adapters::volume::set_mountinfo_for_isolated_test(None);
            }
        }
    });
}

#[test]
fn a_trash_that_moves_nothing_is_retryable_and_records_no_undo() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::deletion::a_trash_that_moves_nothing_is_retryable_and_records_no_undo",
        || {
            let rows: [(Trash, Preset, Option<bool>); 5] = [
                (Trash::FalseWithoutError, Quirks::default, None),
                (Trash::SucceedsWithoutMoving, Quirks::default, None),
                (Trash::SucceedsWithoutMoving, Quirks::vfat, Some(false)),
                (Trash::SucceedsWithoutMoving, Quirks::sshfs, Some(false)),
                (Trash::SucceedsWithoutMoving, Quirks::exfat, Some(false)),
            ];
            for (trash, preset, evicted) in rows {
                let root = tempfile::tempdir().expect("root");
                let folder = root.path().join("folder");
                fs::create_dir(&folder).expect("folder");
                let evicting = Arc::new(Mutex::new(None));
                let (path, outcome) = (folder.clone(), evicting.clone());
                let racer: super::fs_quirks::Racer = Box::new(move || {
                    *outcome.lock().expect("outcome") = Some(super::fs_quirks::evict(&path));
                });
                let _quirks = FsQuirks::install(
                    root.path(),
                    Quirks {
                        trash,
                        after: if evicted.is_some() {
                            vec![(Op::Trash, "folder", 0, racer)]
                        } else {
                            Vec::new()
                        },
                        ..preset()
                    },
                );
                let events = Rc::new(RefCell::new(Vec::new()));
                let emitted = events.clone();
                let _operation = LocalOperationProvider.delete(
                    DeleteRequest {
                        id: OperationRequestId(4060),
                        entries: vec![directory_entry(&folder)],
                        permanent: false,
                    },
                    Rc::new(move |event| emitted.borrow_mut().push(event)),
                );
                wait_for_operation(&events, |event| {
                    matches!(
                        event,
                        OperationEvent::Deleted { .. } | OperationEvent::CompletedWithErrors { .. }
                    )
                });

                let events = events.borrow();
                assert!(
                    matches!(
                        events.last(),
                        Some(OperationEvent::CompletedWithErrors { deleted_locations, retryable_locations, .. })
                            if deleted_locations.is_empty()
                                && retryable_locations == &[Location::local(&folder)]
                    ),
                    "{trash:?}: {:?}",
                    events.last()
                );
                assert!(trashed_items(&events).is_empty());
                assert!(folder.is_dir());
                assert_eq!(
                    *evicting.lock().expect("outcome"),
                    evicted,
                    "{:?}",
                    preset().mount_type
                );
            }
        },
    );
}

const CLOCK_START: Duration = Duration::from_secs(1_800_000_000);

type Holds = Arc<std::sync::Mutex<Vec<HoldGuard>>>;

fn releasing(holds: &Holds) -> Racer {
    let holds = holds.clone();
    Box::new(move || holds.lock().expect("the holds").clear())
}

fn folder_with_an_open_file(path: &Path, holds: &Holds) -> io::Result<()> {
    fs::create_dir(path)?;
    fs::write(path.join("open.txt"), b"open elsewhere")?;
    let held = hold(&path.join("open.txt"));
    holds
        .lock()
        .map_err(|_| io::Error::other("the holds"))?
        .push(held);
    Ok(())
}

fn delete_permanently(folders: &[PathBuf]) -> Vec<OperationEvent> {
    delete(folders, true)
}

fn delete(folders: &[PathBuf], permanent: bool) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    glib::MainContext::default().block_on(run_deletion_in(
        Rc::new(SystemTrash),
        OperationRequestId(1590),
        folders
            .iter()
            .map(|folder| DeletionTarget::from_entry(&directory_entry(folder)))
            .collect(),
        permanent,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
        gio::Cancellable::new(),
    ));
    events.take()
}

fn waited_since(start: SystemTime) -> Result<Duration, Box<dyn Error>> {
    Ok(super::super::copy_engine::clock_for_test().duration_since(start)?)
}

#[derive(Clone, Copy, Debug)]
enum Holding {
    Unnamed,
    Program,
    StrataRead,
    StrataOperation,
    TheDeleteItself,
}

#[derive(Clone, Copy, Debug)]
enum Meanwhile {
    StaysOpen(Holding),
    Closes,
    ClosesAsTheWaitEnds,
    IsSwappedForAnEmptyFolder,
}

const GONE_AFTER_THE_CLOSE: Duration = Duration::from_millis(100);

fn meanwhile_racer(
    meanwhile: Meanwhile,
    folder: &Path,
    holds: &Holds,
    wait_ends: SystemTime,
) -> Option<(Op, &'static str, usize, Racer)> {
    let (holds, folder) = (holds.clone(), folder.to_path_buf());
    match meanwhile {
        Meanwhile::StaysOpen(_) => None,
        Meanwhile::Closes => Some((Op::Unlink, "folder", 0, releasing(&holds))),
        Meanwhile::ClosesAsTheWaitEnds => Some((
            Op::Unlink,
            "folder",
            1,
            Box::new(move || {
                super::super::copy_engine::set_clock_for_test(Some(wait_ends));
                holds.lock().expect("the holds").clear();
                let gone = wait_ends + GONE_AFTER_THE_CLOSE;
                super::super::copy_engine::set_clock_for_test(Some(gone));
            }),
        )),
        Meanwhile::IsSwappedForAnEmptyFolder => Some((
            Op::Unlink,
            "folder",
            0,
            Box::new(move || {
                fs::rename(&folder, folder.with_extension("moved")).expect("move the folder");
                fs::create_dir(&folder).expect("make another folder");
            }),
        )),
    }
}

#[test]
fn a_folder_only_leftovers_keep_goes_once_they_do_within_the_mount_s_wait()
-> Result<(), Box<dyn Error>> {
    use Holding::*;
    use Meanwhile::*;
    let _serial = serial()?;
    let smb = Duration::from_millis(1500);
    let fuse_and_nfs = Duration::from_secs(2);
    let renamed = |holding| {
        [
            StaysOpen(Unnamed),
            StaysOpen(holding),
            Closes,
            ClosesAsTheWaitEnds,
            IsSwappedForAnEmptyFolder,
        ]
    };
    let rows: [(&str, Preset, Duration, &[Meanwhile]); 6] = [
        ("nfs3", Quirks::nfs3, fuse_and_nfs, &renamed(StrataRead)),
        ("nfs42", Quirks::nfs42, fuse_and_nfs, &renamed(Program)),
        ("cifs", Quirks::cifs, smb, &renamed(StrataOperation)),
        (
            "cifs-posix",
            Quirks::cifs_posix,
            smb,
            &[
                StaysOpen(Unnamed),
                StaysOpen(Program),
                Closes,
                IsSwappedForAnEmptyFolder,
            ],
        ),
        (
            "sshfs",
            Quirks::sshfs,
            fuse_and_nfs,
            &renamed(TheDeleteItself),
        ),
        (
            "exfat-fuse",
            Quirks::exfat_fuse,
            fuse_and_nfs,
            &renamed(Unnamed),
        ),
    ];
    for (mount, preset, budget, cases) in rows {
        for &meanwhile in cases {
            let row = format!("{mount}, {meanwhile:?}");
            let start = SystemTime::UNIX_EPOCH + CLOCK_START;
            let _clock = ClockGuard::set(start);
            let root = tempfile::tempdir()?;
            let folder = root.path().join("folder");
            let holds = Holds::default();
            let holding = match meanwhile {
                StaysOpen(holding) => holding,
                _ => Unnamed,
            };
            let scope = FsQuirks::install(
                root.path(),
                Quirks {
                    after: meanwhile_racer(meanwhile, &folder, &holds, start + budget)
                        .into_iter()
                        .collect(),
                    holder: matches!(holding, Program)
                        .then(|| Holder::Program("Text Editor".to_owned())),
                    ..preset()
                },
            );
            folder_with_an_open_file(&folder, &holds)?;
            let real_folder = folder.canonicalize()?;
            let read_by_strata = real_folder.join("open.txt");
            let _read = matches!(holding, StrataRead)
                .then(|| file_activity::hold([&read_by_strata], || {}))
                .transpose()?;
            let _other_operation = matches!(holding, StrataOperation).then(|| {
                file_activity::begin(file_activity::OperationScope {
                    roots: vec![read_by_strata.clone()],
                    folders: Vec::new(),
                })
            });
            let _this_delete = matches!(holding, TheDeleteItself).then(|| {
                file_activity::begin(file_activity::OperationScope {
                    roots: vec![real_folder.clone(), read_by_strata.clone()],
                    folders: Vec::new(),
                })
            });

            let events = delete_permanently(std::slice::from_ref(&folder));

            let waited = waited_since(start)?;
            let deleted = matches!(events.last(), Some(OperationEvent::Deleted { .. }));
            let leftovers_only = |folder: &Path| -> Result<bool, Box<dyn Error>> {
                let left = names(folder)?;
                Ok(left.len() <= 1
                    && left
                        .iter()
                        .all(|name| foreign_leftover(OsStr::new(name)).is_some()))
            };
            match meanwhile {
                Closes | ClosesAsTheWaitEnds => {
                    assert!(deleted, "{row}: {events:?}");
                    assert!(!folder.exists(), "{row}");
                    if let Closes = meanwhile {
                        assert!(waited < budget, "{row}: {waited:?}");
                    }
                }
                StaysOpen(holding) => {
                    assert!(!deleted, "{row}: {events:?}");
                    assert_eq!(waited, budget, "{row}");
                    assert!(leftovers_only(&folder)?, "{row}");
                    let reason = match (holding, mount) {
                        (Program, _) => rust_i18n::t!(
                            "A file in it is still open in %{program}. Close it there and try again.",
                            program = "Text Editor"
                        )
                        .into_owned(),
                        (StrataRead | StrataOperation, _) => crate::i18n::tr(
                            "A file in it is still in use by Strata. Try again when its other operation or search ends.",
                        ),
                        (Unnamed | TheDeleteItself, "cifs") => crate::i18n::tr(
                            "A file in it is still open in a program on this or another computer.",
                        ),
                        (Unnamed | TheDeleteItself, "cifs-posix") => crate::services::io_error_detail(
                            &io::Error::from_raw_os_error(rustix::io::Errno::ACCESS.raw_os_error()),
                        ),
                        (Unnamed | TheDeleteItself, _) => crate::i18n::tr(
                            "A file in it is still open in another program. Close it and try again.",
                        ),
                    };
                    let line = rust_i18n::t!(
                        "Could not delete “%{name}”: %{error}",
                        name = "folder",
                        error = reason
                    );
                    assert!(
                        matches!(
                            events.last(),
                            Some(OperationEvent::CompletedWithErrors { message, .. })
                                if message.contains(&*line)
                        ),
                        "{row}: {events:?}"
                    );
                    let scanned = scope.count(Op::Holders);
                    match holding {
                        StrataRead | StrataOperation => assert_eq!(scanned, 0, "{row}"),
                        Unnamed | Program | TheDeleteItself => assert_eq!(scanned, 1, "{row}"),
                    }
                }
                IsSwappedForAnEmptyFolder => {
                    assert!(!deleted, "{row}: {events:?}");
                    assert!(names(&folder)?.is_empty(), "{row}");
                    assert!(leftovers_only(&folder.with_extension("moved"))?, "{row}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn one_wait_for_open_files_covers_every_folder_of_a_delete_or_trash() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let smb_folders_refused = || Quirks {
        folder_renames_refused_below: Some(Duration::from_secs(1)),
        ..Quirks::cifs()
    };
    let rows: [(&str, Preset, bool, Duration); 2] = [
        ("nfs3, Delete", Quirks::nfs3, true, Duration::from_secs(2)),
        (
            "cifs, Trash",
            smb_folders_refused,
            false,
            Duration::from_millis(1500),
        ),
    ];
    for (row, preset, permanent, budget) in rows {
        let start = SystemTime::UNIX_EPOCH + CLOCK_START;
        let _clock = ClockGuard::set(start);
        let root = tempfile::tempdir()?;
        let _scope = FsQuirks::install(root.path(), preset());
        let holds = Holds::default();
        let folders = [root.path().join("first"), root.path().join("second")];
        for folder in &folders {
            folder_with_an_open_file(folder, &holds)?;
        }

        let events = delete(&folders, permanent);

        assert!(
            matches!(
                events.last(),
                Some(OperationEvent::CompletedWithErrors { deleted_locations, .. })
                    if deleted_locations.is_empty()
            ),
            "{row}: {events:?}"
        );
        assert!(folders.iter().all(|folder| folder.is_dir()), "{row}");
        assert_eq!(waited_since(start)?, budget, "{row}");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Hidden {
    Leftover { closed_after: Option<Op> },
    LeftoverBesideAnEarlierOne,
    NamedLikeOne,
}

#[test]
fn a_recursive_delete_never_unlinks_another_program_s_leftover() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let leftover = |closed_after| Hidden::Leftover { closed_after };
    let earlier = ".__smbfile_silly1";
    let rows: [(&str, Preset, Hidden); 10] = [
        ("nfs3", Quirks::nfs3, leftover(None)),
        ("nfs3", Quirks::nfs3, leftover(Some(Op::Unlink))),
        ("nfs3", Quirks::nfs3, leftover(Some(Op::List))),
        ("cifs", Quirks::cifs, leftover(None)),
        ("cifs", Quirks::cifs, leftover(Some(Op::Unlink))),
        ("cifs", Quirks::cifs, Hidden::LeftoverBesideAnEarlierOne),
        ("sshfs", Quirks::sshfs, leftover(None)),
        ("sshfs", Quirks::sshfs, leftover(Some(Op::Unlink))),
        ("ext4", Quirks::ext4, Hidden::NamedLikeOne),
        ("exfat-fuse", Quirks::exfat_fuse, Hidden::NamedLikeOne),
    ];
    for (mount, preset, hidden) in rows {
        let row = format!("{mount}, {hidden:?}");
        let start = SystemTime::UNIX_EPOCH + CLOCK_START;
        let _clock = ClockGuard::set(start);
        let root = tempfile::tempdir()?;
        let folder = root.path().join("folder");
        let holds = Holds::default();
        let closed_after = match hidden {
            Hidden::Leftover { closed_after } => closed_after,
            Hidden::LeftoverBesideAnEarlierOne | Hidden::NamedLikeOne => None,
        };
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                after: closed_after
                    .map(|op| (op, "folder", 0, releasing(&holds)))
                    .into_iter()
                    .collect(),
                ..preset()
            },
        );
        match hidden {
            Hidden::Leftover { .. } | Hidden::LeftoverBesideAnEarlierOne => {
                folder_with_an_open_file(&folder, &holds)?;
                if let Hidden::LeftoverBesideAnEarlierOne = hidden {
                    fs::write(folder.join(earlier), b"earlier")?;
                }
                let dir = open_local_parent_directory(&folder)?;
                super::super::fs_ops::fs().unlink(
                    super::super::fs_ops::Entry::new(&dir, OsStr::new("open.txt")),
                    rustix::fs::AtFlags::empty(),
                )?;
            }
            Hidden::NamedLikeOne => {
                fs::create_dir(&folder)?;
                fs::write(folder.join(".nfs000000000000005e000000de"), b"ordinary")?;
            }
        }
        let hidden_names = names(&folder)?;
        if let Hidden::LeftoverBesideAnEarlierOne = hidden {
            assert_eq!(hidden_names.len(), 2, "{row}: {hidden_names:?}");
            assert_eq!(fs::read(folder.join(earlier))?, b"earlier", "{row}");
        } else {
            assert_eq!(hidden_names.len(), 1, "{row}: {hidden_names:?}");
        }
        fs::write(folder.join("ordinary.txt"), b"ordinary")?;
        scope.clear_calls();

        let events = delete_permanently(std::slice::from_ref(&folder));

        let unlinked = scope
            .calls()
            .into_iter()
            .filter(|call| call.op == Op::Unlink)
            .flat_map(|call| call.names)
            .collect::<Vec<_>>();
        assert!(
            unlinked.contains(&"ordinary.txt".into()),
            "{row}: {unlinked:?}"
        );
        if let Hidden::NamedLikeOne = hidden {
            assert!(!folder.exists(), "{row}: {events:?}");
            continue;
        }
        assert!(
            hidden_names
                .iter()
                .all(|name| !unlinked.contains(&name.into())),
            "{row}: {unlinked:?}"
        );
        if closed_after.is_some() {
            assert!(
                matches!(events.last(), Some(OperationEvent::Deleted { .. })),
                "{row}: {events:?}"
            );
            assert!(!folder.exists(), "{row}");
        } else {
            assert!(
                matches!(
                    events.last(),
                    Some(OperationEvent::CompletedWithErrors { .. })
                ),
                "{row}: {events:?}"
            );
            assert_eq!(names(&folder)?, hidden_names, "{row}");
        }
    }
    Ok(())
}
