use super::*;
use crate::adapters::local_operations::{
    LocalDeleteJob, LocalDeleteQueue, LocalDeleteRoot, parallel_delete_local_blocking_with_workers,
    parallel_delete_local_with, process_local_delete_job, retry_local_open,
    run_local_delete_workers,
};
use std::sync::{Barrier, atomic::Ordering, mpsc};

fn delete_root(path: &Path) -> Result<LocalDeleteRoot, Box<dyn Error>> {
    Ok(LocalDeleteRoot {
        parent: Arc::new(super::super::open_local_parent_directory(
            path.parent().ok_or("no parent")?,
        )?),
        name: path.file_name().ok_or("no name")?.to_owned(),
        expected: None,
    })
}

fn enqueue_root(queue: &Arc<LocalDeleteQueue>, root: LocalDeleteRoot) {
    queue.enqueue(LocalDeleteJob::Entry {
        parent: root.parent,
        name: root.name,
        expected: root.expected,
        guard: None,
        completion: None,
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
        )?;
        assert!(!target.exists());
        assert_eq!(fs::read(sentinel)?, b"keep");
    }
    Ok(())
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

    let summary = deletion_error_summary(&errors);

    assert!(summary.starts_with("10 items could not be deleted"));
    assert!(summary.contains("• item-1: denied"));
    assert!(summary.contains("• item-8: denied"));
    assert!(!summary.contains("• item-9: denied"));
    assert!(summary.ends_with("…and 2 more"));
    assert!(
        operation_error_summary(&errors[..1], "restored")
            .starts_with("1 item could not be restored")
    );
}

#[test]
fn rotational_deletes_cap_parallelism_without_disabling_it() {
    assert_eq!(super::bounded_local_delete_worker_count(0, false), 1);
    assert_eq!(super::bounded_local_delete_worker_count(1, true), 1);
    assert_eq!(super::bounded_local_delete_worker_count(8, true), 1);
    assert_eq!(super::bounded_local_delete_worker_count(8, false), 2);
}

#[test]
fn a_backend_without_trash_support_gets_an_actionable_message() {
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "trash not supported");

    let trash_message = deletion_error_message("share-folder", false, &error);
    assert!(trash_message.contains("doesn't support Trash"));
    assert!(trash_message.contains("Delete permanently instead"));

    let permanent_message = deletion_error_message("share-folder", true, &error);
    assert!(!permanent_message.contains("Trash"));
    assert!(permanent_message.contains("trash not supported"));
}

#[test]
fn a_trash_attempt_that_fails_as_unsupported_is_retryable() {
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "trash not supported");
    assert!(is_trash_unsupported_failure(false, &error));
}

#[test]
fn an_already_permanent_delete_failure_is_never_retryable() {
    // Nothing left to fall back to if a *permanent* delete itself failed
    // with `NotSupported` -- retrying it the same way would just fail again.
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "trash not supported");
    assert!(!is_trash_unsupported_failure(true, &error));
}

#[test]
fn an_unrelated_trash_failure_is_not_retryable() {
    let error = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "access denied");
    assert!(!is_trash_unsupported_failure(false, &error));
}

#[test]
fn other_deletion_failures_keep_the_raw_error() {
    let error = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "access denied");

    let message = deletion_error_message("secret.txt", false, &error);

    assert_eq!(message, "secret.txt: access denied");
}

#[test]
fn cancelling_between_deletions_reports_completed_and_unattempted_items()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
fn permanent_delete_removes_a_symlink_standing_in_for_a_directory_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
fn permanent_delete_of_a_symlink_through_an_alias_keeps_its_referent() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
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
    let delete_root = super::LocalDeleteRoot {
        parent: Arc::new(parent),
        name: root.file_name().ok_or("no name")?.to_owned(),
        expected: None,
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
