// SPDX-License-Identifier: MIT

use super::*;

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
    assert!(super::super::super::was_cancelled(&error));
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
    let parent = super::super::open_local_parent_directory(root.parent().ok_or("no parent")?)?;
    let delete_root = super::super::PinnedEntry {
        parent: Arc::new(parent),
        name: root.file_name().ok_or("no name")?.to_owned(),
        identity: None,
    };
    let context = glib::MainContext::default();
    let error = context
        .block_on(super::super::parallel_delete_local(
            vec![delete_root],
            cancellable,
        ))
        .expect_err("cancelled delete must return error");

    assert!(super::super::was_cancelled(&error));
    assert!(root.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}
