// SPDX-License-Identifier: MIT

use super::*;

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
