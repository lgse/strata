// SPDX-License-Identifier: MIT

use super::*;

fn drive_until_rename_settles(events: &Rc<RefCell<Vec<OperationEvent>>>) {
    wait_for_operation(events, |event| {
        matches!(
            event,
            OperationEvent::Renamed { .. }
                | OperationEvent::Failed { .. }
                | OperationEvent::Cancelled { .. }
        )
    });
}

#[test]
fn undoing_a_move_returns_each_item_to_its_original_directory() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let moved = archive.join("report.txt");
    fs::write(&moved, b"contents")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(40),
            items: vec![UndoMoveItem {
                record: MoveRecord {
                    original: Location::local(origin.join("report.txt")),
                    current: Location::local(&moved),
                },
                conflict: TransferConflict::FailIfExists,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!moved.exists());
    assert_eq!(fs::read(origin.join("report.txt"))?, b"contents");
    Ok(())
}

#[test]
fn undoing_a_move_stops_at_an_unconfirmed_conflict() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let blocked = origin.join("report.txt");
    fs::write(&blocked, b"newer")?;
    let first = archive.join("notes.txt");
    let second = archive.join("report.txt");
    fs::write(&first, b"first")?;
    fs::write(&second, b"second")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(41),
            items: vec![
                UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(origin.join("notes.txt")),
                        current: Location::local(&first),
                    },
                    conflict: TransferConflict::FailIfExists,
                },
                UndoMoveItem {
                    record: MoveRecord {
                        original: Location::local(&blocked),
                        current: Location::local(&second),
                    },
                    conflict: TransferConflict::FailIfExists,
                },
            ],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    let completed = match events.borrow().last() {
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) => completed_locations.clone(),
        other => panic!("expected a transfer failure, got {other:?}"),
    };
    assert_eq!(completed, vec![Location::local(&first)]);
    assert_eq!(fs::read(&blocked)?, b"newer");
    assert_eq!(fs::read(&second)?, b"second");
    assert_eq!(fs::read(origin.join("notes.txt"))?, b"first");
    Ok(())
}

#[test]
fn a_confirmed_undo_conflict_replaces_the_newer_item() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("origin");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let original = origin.join("report.txt");
    let moved = archive.join("report.txt");
    fs::write(&original, b"newer")?;
    fs::write(&moved, b"moved")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(42),
            items: vec![UndoMoveItem {
                record: MoveRecord {
                    original: Location::local(&original),
                    current: Location::local(&moved),
                },
                conflict: TransferConflict::ReplaceExisting,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    drive_until_transfer_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!moved.exists());
    assert_eq!(fs::read(&original)?, b"moved");
    Ok(())
}

#[test]
fn rename_undo_restores_a_native_name_byte_for_byte() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    let original_name = OsString::from_vec(b"original-\xff.txt".to_vec());
    let original = original_parent.join(&original_name);
    let current = current_parent.join("renamed.txt");
    fs::write(&current, b"contents")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_rename(
        UndoRenameRequest {
            id: OperationRequestId(43),
            current: Location::local(&current),
            original: Location::local(&original),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_rename_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Renamed { .. })
    ));
    assert!(!current.exists());
    assert_eq!(fs::read(&original)?, b"contents");
    assert_eq!(
        original_name.as_encoded_bytes(),
        original
            .file_name()
            .expect("original test path should have a file name")
            .as_encoded_bytes()
    );
    Ok(())
}

#[test]
fn rename_undo_refuses_an_occupied_destination_and_can_retry() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    let original = original_parent.join("item.txt");
    let current = current_parent.join("renamed.txt");
    fs::write(&original, b"occupant")?;
    fs::write(&current, b"renamed")?;
    let request = || UndoRenameRequest {
        id: OperationRequestId(44),
        current: Location::local(&current),
        original: Location::local(&original),
    };

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    {
        let _operation = LocalOperationProvider.undo_rename(
            request(),
            Rc::new(move |event| emitted.borrow_mut().push(event)),
        );
        drive_until_rename_settles(&events);
    }
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Failed { .. })
    ));
    assert_eq!(fs::read(&original)?, b"occupant");
    assert_eq!(fs::read(&current)?, b"renamed");

    fs::remove_file(&original)?;
    events.borrow_mut().clear();
    let emitted = events.clone();
    let _operation = LocalOperationProvider.undo_rename(
        request(),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drive_until_rename_settles(&events);
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Renamed { .. })
    ));
    assert!(!current.exists());
    assert_eq!(fs::read(&original)?, b"renamed");
    Ok(())
}

fn undo_one(
    id: u64,
    original: &Path,
    current: &Path,
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.undo_move(
        UndoMoveRequest {
            id: OperationRequestId(id),
            items: vec![UndoMoveItem {
                record: MoveRecord {
                    original: Location::local(original),
                    current: Location::local(current),
                },
                conflict,
            }],
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    (operation, events)
}

#[test]
fn undoing_a_cross_volume_move_keeps_the_item_when_the_removable_flush_fails()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("usb");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let current = archive.join("report.txt");
    fs::write(&current, b"contents")?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other), true);

    let (_operation, events) = undo_one(
        97,
        &origin.join("report.txt"),
        &current,
        TransferConflict::FailIfExists,
    );
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::TransferFailed { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(&current)?, b"contents");
    assert_eq!(fs::read(origin.join("report.txt"))?, b"contents");
    Ok(())
}

#[test]
fn undoing_a_cross_volume_move_flushes_before_deleting_the_moved_item() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let origin = root.path().join("usb");
    let archive = root.path().join("archive");
    fs::create_dir_all(&origin)?;
    fs::create_dir_all(&archive)?;
    let current = archive.join("report.txt");
    fs::write(&current, b"contents")?;
    let during = watch_source_during_sync(&current);
    let _guard = RemovableFlushGuard::install(root.path(), None, true);

    let (_operation, events) = undo_one(
        98,
        &origin.join("report.txt"),
        &current,
        TransferConflict::FailIfExists,
    );
    pump_until_transfer(&events);
    assert_eq!(
        during
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .first()
            .copied(),
        Some(true)
    );

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!current.exists());
    assert_eq!(fs::read(origin.join("report.txt"))?, b"contents");
    Ok(())
}

#[test]
fn cancelled_rename_undo_reports_cancellation_without_moving_the_item() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let original_parent = root.path().join("original");
    let current_parent = root.path().join("current");
    fs::create_dir_all(&original_parent)?;
    fs::create_dir_all(&current_parent)?;
    let original = original_parent.join("item.txt");
    let current = current_parent.join("renamed.txt");
    fs::write(&current, b"renamed")?;
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.undo_rename(
        UndoRenameRequest {
            id: OperationRequestId(45),
            current: Location::local(&current),
            original: Location::local(&original),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drop(operation);
    drive_until_rename_settles(&events);

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(current.exists());
    assert!(!original.exists());
    Ok(())
}
