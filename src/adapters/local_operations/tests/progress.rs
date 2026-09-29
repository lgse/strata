// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn completed_gio_result_wins_a_cancellation_race() {
    let context = glib::MainContext::new();
    let cancellable = gio::Cancellable::new();
    let cancel_after_result = cancellable.clone();
    let file = gio::File::for_path("/fixture");

    let result = context.block_on(await_cancellable(
        &file,
        &cancellable,
        move |_, _, result| {
            result.resolve(Ok::<_, glib::Error>(()));
            cancel_after_result.cancel();
        },
    ));

    assert!(result.is_ok());
}

#[test]
fn local_task_batches_wait_for_every_started_task_after_an_operation_error()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let context = glib::MainContext::default();
    let finished = Rc::new(Cell::new(false));
    let finished_by_task = finished.clone();
    let failed = context.spawn_local(async { Err::<(), _>(io_error("injected failure")) });
    let delayed = context.spawn_local(async move {
        glib::timeout_future(Duration::from_millis(10)).await;
        finished_by_task.set(true);
        Ok::<(), glib::Error>(())
    });

    let results = context.block_on(join_local_tasks(vec![
        ("failed", failed),
        ("delayed", delayed),
    ]));

    assert!(matches!(&results[0], ("failed", Ok(Err(_)))));
    assert!(matches!(&results[1], ("delayed", Ok(Ok(())))));
    assert!(finished.get());
    Ok(())
}

#[test]
fn local_task_batches_keep_panics_attributed_to_their_input() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let context = glib::MainContext::default();
    let panicked: glib::JoinHandle<u8> =
        context.spawn_local(async { panic!("injected task panic") });
    let completed = context.spawn_local(async { 2_u8 });

    let results = context.block_on(join_local_tasks(vec![
        ("panicked", panicked),
        ("completed", completed),
    ]));

    assert!(matches!(&results[0], ("panicked", Err(_))));
    assert!(matches!(&results[1], ("completed", Ok(2))));
    Ok(())
}

#[test]
fn transfer_progress_aggregates_file_bytes_without_emitting_per_file() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let tracker = TransferProgressTracker::new(
        OperationRequestId(24),
        Some(150),
        Some(2),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    tracker.emit();
    let first = tracker.begin_file("first.iso".to_owned(), false);
    first.finish(Some(100));
    assert_eq!(events.borrow().len(), 1);
    tracker.finish_item(0, Some(100), 0, Some(1), None);

    let second = tracker.begin_file("second.iso".to_owned(), false);
    second.finish(Some(50));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferProgress {
            completed_items: 1,
            completed_files: 2,
            total_files: Some(2),
            current_file: Some(name),
            transferred_bytes: 150,
            total_bytes: Some(150),
            ..
        }) if name == "second.iso"
    ));
    Ok(())
}

#[test]
fn copy_byte_progress_crosses_worker_threads_without_flooding_the_main_loop()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let tracker = TransferProgressTracker::new(
        OperationRequestId(29),
        Some(100),
        Some(1),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    tracker.emit();
    let file = tracker.begin_file("large.iso".to_owned(), true);
    let reporter = file.reporter().expect("byte progress reporter");

    thread::spawn(move || reporter.record(40, 100))
        .join()
        .expect("copy progress worker");
    glib::MainContext::default().block_on(glib::timeout_future(
        TRANSFER_PROGRESS_INTERVAL + Duration::from_millis(10),
    ));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 0,
            completed_files: 0,
            transferred_bytes: 40,
            total_bytes: Some(100),
            ..
        }
    )));
    file.finish(Some(100));
    Ok(())
}

#[test]
fn fat32_copy_errors_without_english_size_text_skip_an_uncancellable_flush() {
    let error = glib::Error::new(gio::IOErrorEnum::Failed, "Error splicing file: EFBIG");
    let (message, skip_flush) = copy_failure_on_fat32(&error, true);
    assert!(skip_flush);
    let message = message.expect("FAT32 write failed");
    assert!(message.contains("EFBIG"));
    assert!(message.contains("wait for safe eject"));
    let (message, skip_flush) = copy_failure_on_fat32(&error, false);
    assert!(!skip_flush);
    assert!(
        !message
            .expect("non-FAT32 write failed")
            .contains("safe eject")
    );
}

#[test]
fn fat32_size_check_rejects_oversized_nested_file_before_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let nested = root.path().join("sources").join("folder");
    fs::create_dir_all(&nested)?;
    let oversized = nested.join("ubuntu.iso");
    fs::File::create(&oversized)?.set_len(FAT32_MAX_FILE_SIZE + 1)?;
    let source = gio::File::for_path(root.path().join("sources"));
    let cancellable = gio::Cancellable::new();
    let error = glib::MainContext::default()
        .block_on(transfer_sizes(
            std::slice::from_ref(&source),
            &cancellable,
            fat32_file_size_limit(Some("vfat")),
        ))
        .expect_err("FAT32 cannot store an individual file larger than 4 GiB");
    assert!(error.message().contains(" is too large for a FAT32 drive "));
    assert!(error.to_string().contains("ubuntu.iso"));
    assert!(error.to_string().contains("exFAT"));
    assert!(fat32_file_size_limit(Some("msdos")).is_some());
    assert!(fat32_file_size_limit(Some("exfat")).is_none());

    let (_, bytes, counts, total_files, _) =
        glib::MainContext::default().block_on(transfer_sizes(
            &[source],
            &cancellable,
            fat32_file_size_limit(Some("exfat")),
        ))?;
    assert_eq!(bytes, Some(FAT32_MAX_FILE_SIZE + 1));
    assert_eq!(counts, vec![Some(1)]);
    assert_eq!(total_files, Some(1));
    Ok(())
}

#[test]
fn copying_a_file_emits_bytes_before_item_completion() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let destination = root.path().join("destination");
    let contents = vec![0x5a; 1024 * 1024];
    fs::write(&source, &contents)?;
    fs::create_dir(&destination)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(25),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. }
                | OperationEvent::Cancelled { .. }
                | OperationEvent::TransferFailed { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 0,
            completed_files: 1,
            total_files: Some(1),
            transferred_bytes,
            total_bytes: Some(total_bytes),
            ..
        } if *transferred_bytes > 0 && *total_bytes == contents.len() as u64
    )));
    assert_eq!(fs::read(destination.join("source.bin"))?, contents);
    Ok(())
}

#[test]
fn nested_copy_reports_files_before_the_selected_folder_finishes() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::create_dir(&destination)?;
    for index in 0..32 {
        fs::write(
            source.join(format!("file-{index:02}.txt")),
            index.to_string(),
        )?;
    }
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(26),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 0,
            completed_files,
            total_files: Some(32),
            ..
        } if *completed_files > 0
    )));
    for index in 0..32 {
        assert_eq!(
            fs::read_to_string(destination.join(format!("source/file-{index:02}.txt")))?,
            index.to_string()
        );
    }
    Ok(())
}

#[test]
fn copying_many_selected_files_completes_the_bounded_bulk_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::create_dir(&destination)?;
    let mut items = Vec::new();
    let mut expected = Vec::new();
    for index in 0..12 {
        let path = source.join(format!("file-{index:02}.txt"));
        fs::write(&path, index.to_string())?;
        expected.push(Location::local(&path));
        items.push(PasteItem {
            source: Location::local(path),
            conflict: TransferConflict::FailIfExists,
        });
    }

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(27),
            destination: Location::local(&destination),
            items,
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { locations, .. }) if locations == &expected
    ));
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(
                event,
                OperationEvent::TransferProgress {
                    created_location: Some(_),
                    ..
                }
            ))
            .count(),
        expected.len()
    );
    for index in 0..12 {
        assert_eq!(
            fs::read_to_string(destination.join(format!("file-{index:02}.txt")))?,
            index.to_string()
        );
    }
    Ok(())
}

#[test]
fn bulk_copy_preserves_symlinks_and_one_file_directories() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source)?;
    fs::create_dir(&destination)?;

    let mut items = Vec::new();
    let mut expected = Vec::new();
    for index in 0..9 {
        let path = source.join(format!("file-{index:02}.txt"));
        fs::write(&path, index.to_string())?;
        expected.push(Location::local(&path));
        items.push(PasteItem {
            source: Location::local(path),
            conflict: TransferConflict::FailIfExists,
        });
    }
    let folder = source.join("folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join("nested.txt"), b"nested")?;
    expected.push(Location::local(&folder));
    items.push(PasteItem {
        source: Location::local(&folder),
        conflict: TransferConflict::FailIfExists,
    });
    let link = source.join("link");
    std::os::unix::fs::symlink("file-00.txt", &link)?;
    expected.push(Location::local(&link));
    items.push(PasteItem {
        source: Location::local(&link),
        conflict: TransferConflict::FailIfExists,
    });

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(28),
            destination: Location::local(&destination),
            items,
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Pasted { .. } | OperationEvent::TransferFailed { .. }
        )
    }) {
        glib::MainContext::default().iteration(true);
    }

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { locations, .. }) if locations == &expected
    ));
    assert_eq!(fs::read(destination.join("folder/nested.txt"))?, b"nested");
    assert_eq!(
        fs::read_link(destination.join("link"))?,
        PathBuf::from("file-00.txt")
    );
    Ok(())
}

#[test]
fn cancelling_between_moves_reports_completed_and_unattempted_sources() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-cancel-test-{unique}"));
    let sources = root.join("sources");
    let destination = root.join("destination");
    let first = sources.join("first.txt");
    let second = sources.join("second.txt");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let operation = Rc::new(RefCell::new(None::<LoadHandle>));
    let emitted = events.clone();
    let operation_for_emit = operation.clone();
    let handle = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(8),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&first),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(&second),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: true,
        },
        Rc::new(move |event| {
            let cancel = matches!(
                event,
                OperationEvent::TransferProgress {
                    completed_items: 1,
                    ..
                }
            );
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
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 1,
            transferred_bytes: 5,
            total_bytes: Some(11),
            ..
        }
    )));
    assert_eq!(result.completed, [Location::local(&first)]);
    assert!(result.failed.is_empty());
    assert_eq!(result.not_attempted, [Location::local(&second)]);
    assert!(destination.join("first.txt").exists());
    assert!(!first.exists());
    assert!(second.exists());

    fs::remove_dir_all(root)?;
    Ok(())
}
