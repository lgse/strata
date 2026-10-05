// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn transfers_into_the_same_location_or_a_descendant_are_noops() {
    let source = gio::File::for_path("/fixture/source");
    let parent = gio::File::for_path("/fixture");
    let same_target = parent.child("source");
    let descendant = gio::File::for_path("/fixture/source/nested");
    let descendant_target = descendant.child("source");
    let unrelated = gio::File::for_path("/elsewhere");
    let unrelated_target = unrelated.child("source");

    assert!(transfer_is_noop(&source, &parent, &same_target));
    assert!(transfer_is_noop(&source, &source, &source.child("source")));
    assert!(transfer_is_noop(&source, &descendant, &descendant_target));
    assert!(!transfer_is_noop(&source, &unrelated, &unrelated_target));
}

#[test]
fn each_transfer_item_keeps_its_own_conflict_decision() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-conflict-decisions-test-{unique}"));
    let sources = root.join("sources");
    let destination = root.join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(sources.join("late.txt"), b"new late item")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(destination.join("late.txt"), b"late arrival")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(1),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(sources.join("replace.txt")),
                    conflict: TransferConflict::ReplaceExisting,
                },
                PasteItem {
                    source: Location::local(sources.join("late.txt")),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: true,
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
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) if completed_locations == &[Location::local(sources.join("replace.txt"))]
    ));
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"new replacement"
    );
    assert_eq!(fs::read(destination.join("late.txt"))?, b"late arrival");
    assert!(!sources.join("replace.txt").exists());
    assert!(sources.join("late.txt").exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cutting_in_the_same_folder_remains_a_noop() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let file = destination.join("document.txt");
    let directory = destination.join("folder");
    fs::write(&file, b"content")?;
    fs::create_dir_all(&directory)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(14),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&file),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(&directory),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: true,
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
    assert!(file.exists());
    assert!(directory.is_dir());
    assert!(!destination.join("document (1).txt").exists());
    assert!(!destination.join("folder (1)").exists());
    Ok(())
}

#[test]
fn keeping_both_preserves_transfer_noops() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let nested = source.join("nested");
    fs::create_dir_all(&nested)?;
    fs::write(source.join("report.txt"), b"original")?;

    for moving in [false, true] {
        for destination in [&source, &nested] {
            let created = run_paste_collecting_created(PasteRequest {
                id: OperationRequestId(77),
                destination: Location::local(destination),
                items: vec![PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::KeepBoth,
                }],
                move_sources: moving,
            })?;
            assert!(created.into_iter().flatten().next().is_none());
            assert_eq!(fs::read_dir(&source)?.count(), 2);
            assert_eq!(fs::read_dir(&nested)?.count(), 0);
            assert_eq!(fs::read(source.join("report.txt"))?, b"original");
        }
    }
    Ok(())
}

#[test]
fn keeping_both_in_a_cross_folder_paste_generates_a_unique_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let source = source_dir.join("report.txt");
    let destination = root.path().join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(74),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::KeepBoth,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("report (1).txt"))]
    );
    assert_eq!(fs::read(destination.join("report.txt"))?, b"existing");
    assert_eq!(fs::read(destination.join("report (1).txt"))?, b"incoming");
    assert!(source.exists());
    Ok(())
}

#[test]
fn keeping_both_while_moving_renames_the_destination_instead_of_replacing_it()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let source = source_dir.join("report.txt");
    let destination = root.path().join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(75),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::KeepBoth,
        }],
        move_sources: true,
    })?;

    assert!(created.into_iter().flatten().next().is_none());
    assert_eq!(fs::read(destination.join("report.txt"))?, b"existing");
    assert_eq!(fs::read(destination.join("report (1).txt"))?, b"incoming");
    assert!(!source.exists());
    Ok(())
}

#[test]
fn mixed_conflict_choices_apply_independently_across_a_multi_item_paste()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("new.txt"), b"brand new")?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(sources.join("keep.txt"), b"new keep")?;
    fs::write(destination.join("keep.txt"), b"old keep")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(76),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(sources.join("new.txt")),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(sources.join("replace.txt")),
                    conflict: TransferConflict::ReplaceExisting,
                },
                PasteItem {
                    source: Location::local(sources.join("keep.txt")),
                    conflict: TransferConflict::KeepBoth,
                },
            ],
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
    assert_eq!(fs::read(destination.join("new.txt"))?, b"brand new");
    assert_eq!(
        fs::read(destination.join("replace.txt"))?,
        b"new replacement"
    );
    assert_eq!(fs::read(destination.join("keep.txt"))?, b"old keep");
    assert_eq!(fs::read(destination.join("keep (1).txt"))?, b"new keep");
    Ok(())
}

#[test]
fn pasting_onto_itself_with_replace_is_a_noop() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("todo.txt");
    fs::write(&source, b"existing\n")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(32),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::ReplaceExisting,
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
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert!(!destination.join("todo (1).txt").exists());
    Ok(())
}

#[test]
fn pasting_onto_itself_with_keep_both_creates_numbered_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("todo.txt");
    fs::write(&source, b"existing\n")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(33),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::KeepBoth,
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
    assert!(source.exists());
    assert_eq!(fs::read(&source)?, b"existing\n");
    assert_eq!(fs::read(destination.join("todo (1).txt"))?, b"existing\n");
    Ok(())
}
