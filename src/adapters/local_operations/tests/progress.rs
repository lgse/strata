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
fn transfer_progress_aggregates_completed_and_in_flight_file_bytes() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let tracker = TransferProgressTracker::new(
        OperationRequestId(24),
        Some(150),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    let first = tracker.begin_file();
    let mut first_callback = first.callback();
    first_callback(25, 100);
    first_callback(100, 100);
    first.finish();
    tracker.finish_item(0, Some(100), None);

    let second = tracker.begin_file();
    let mut second_callback = second.callback();
    second_callback(10, 50);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed_items: 0,
            transferred_bytes: 25,
            total_bytes: Some(150),
            ..
        }
    )));
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferProgress {
            completed_items: 1,
            transferred_bytes: 110,
            total_bytes: Some(150),
            ..
        })
    ));
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
            transferred_bytes,
            total_bytes: Some(total_bytes),
            ..
        } if *transferred_bytes > 0 && *total_bytes == contents.len() as u64
    )));
    assert_eq!(fs::read(destination.join("source.bin"))?, contents);
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
