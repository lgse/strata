// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn moving_a_directory_falls_back_to_a_safe_copy_when_the_move_would_recurse()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-fallback-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    fs::write(source.join("nested/child.txt"), b"child")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        always_would_recurse(),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    assert_eq!(fs::read(target.join("nested/child.txt"))?, b"child");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_non_would_recurse_move_failure_is_returned_without_falling_back() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-real-failure-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|_, _, _| {
            Box::pin(async {
                Err(glib::Error::new(
                    gio::IOErrorEnum::PermissionDenied,
                    "injected permission failure",
                ))
            })
        }),
    ));

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::PermissionDenied)));
    assert_eq!(fs::read(source.join("top.txt"))?, b"top");
    assert!(!target.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_successful_move_attempt_is_used_without_falling_back_to_copy() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-move-success-test-{unique}"));
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("top.txt"), b"top")?;

    let result = glib::MainContext::default().block_on(move_local_with(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        Rc::new(|source, target, _| {
            Box::pin(async move {
                fs::rename(
                    source.path().expect("native source"),
                    target.path().expect("native target"),
                )
                .map_err(super::io_error)
            })
        }),
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("top.txt"))?, b"top");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_plain_move_relocates_the_entry_via_the_hardened_rename_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"payload")?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok());
    assert!(!source.exists());
    assert_eq!(fs::read(target)?, b"payload");
    Ok(())
}

#[test]
fn moving_a_directory_into_its_own_child_fails_instead_of_deleting_it() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("nested"))?;
    fs::write(source.join("top.txt"), b"top")?;
    let target = source.join("nested").join("moved-source");

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_err());
    assert_eq!(fs::read(source.join("top.txt"))?, b"top");
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_sources_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    fs::create_dir(&actual_parent)?;
    fs::write(actual_parent.join("source.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;
    let target = root.path().join("target.txt");

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(linked_parent.join("source.txt")),
        gio::File::for_path(&target),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!actual_parent.join("source.txt").exists());
    assert_eq!(fs::read(target)?, b"keep");
    Ok(())
}

#[test]
fn move_accepts_a_symlink_in_the_destinations_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let actual_destination = root.path().join("actual");
    let linked_destination = root.path().join("linked");
    fs::create_dir(&actual_destination)?;
    fs::write(&source, b"keep")?;
    std::os::unix::fs::symlink(&actual_destination, &linked_destination)?;

    let result = glib::MainContext::default().block_on(move_local(
        gio::File::for_path(&source),
        gio::File::for_path(linked_destination.join("target.txt")),
        gio::Cancellable::new(),
        None,
    ));

    assert!(result.is_ok(), "{result:?}");
    assert!(!source.exists());
    assert_eq!(fs::read(actual_destination.join("target.txt"))?, b"keep");
    Ok(())
}

fn paste_move(
    id: u64,
    destination: &Path,
    source: &Path,
    conflict: TransferConflict,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(id),
            destination: Location::local(destination),
            items: vec![PasteItem {
                source: Location::local(source),
                conflict,
            }],
            move_sources: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    (operation, events)
}

#[test]
fn cross_volume_move_keeps_the_source_when_the_removable_flush_fails() -> Result<(), Box<dyn Error>>
{
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"keep-me")?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other), true);

    let (_operation, events) =
        paste_move(90, &destination, &source, TransferConflict::FailIfExists);
    pump_until_transfer(&events);

    let observed = sync_probe_observations();
    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::TransferFailed { .. })
        ),
        "events={:?} syncs={} source_exists={}",
        events.borrow(),
        observed.len(),
        source.exists()
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(&source)?, b"keep-me");
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"keep-me");
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}

#[test]
fn cross_volume_move_flushes_the_removable_destination_before_deleting_the_source()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"moved")?;
    let during = watch_source_during_sync(&source);
    let _guard = RemovableFlushGuard::install(root.path(), None, true);

    let (_operation, events) =
        paste_move(91, &destination, &source, TransferConflict::FailIfExists);
    pump_until_transfer(&events);
    assert_eq!(
        during
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .first()
            .copied(),
        Some(true),
        "the destination flush must start before the source is deleted"
    );

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Pasted { .. })
    ));
    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"moved");
    Ok(())
}

#[test]
fn replacing_move_onto_removable_media_keeps_the_source_when_the_flush_fails()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("notes.txt"), b"old")?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other), false);

    let (_operation, events) =
        paste_move(92, &destination, &source, TransferConflict::ReplaceExisting);
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
    assert_eq!(fs::read(&source)?, b"incoming");
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"incoming");
    Ok(())
}

#[test]
fn replacing_move_flushes_the_removable_destination_before_deleting_the_source()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("notes.txt"), b"old")?;
    let during = watch_source_during_sync(&source);
    let _guard = RemovableFlushGuard::install(root.path(), None, false);

    let (_operation, events) =
        paste_move(93, &destination, &source, TransferConflict::ReplaceExisting);
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
    assert!(!source.exists());
    assert_eq!(fs::read(destination.join("notes.txt"))?, b"incoming");
    Ok(())
}

#[test]
fn merging_move_onto_removable_media_keeps_the_source_when_the_flush_fails()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(source.join("folder"))?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(source.join("folder/incoming.txt"), b"new")?;
    fs::write(destination.join("folder/stays.txt"), b"keep")?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other), false);

    let (_operation, events) = paste_move(
        94,
        &destination,
        &source.join("folder"),
        TransferConflict::Merge,
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
    assert_eq!(fs::read(source.join("folder/incoming.txt"))?, b"new");
    assert_eq!(fs::read(destination.join("folder/incoming.txt"))?, b"new");
    assert_eq!(fs::read(destination.join("folder/stays.txt"))?, b"keep");
    Ok(())
}

#[test]
fn merging_move_flushes_the_removable_destination_before_deleting_the_source()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(source.join("folder"))?;
    fs::create_dir_all(destination.join("folder"))?;
    fs::write(source.join("folder/incoming.txt"), b"new")?;
    fs::write(destination.join("folder/stays.txt"), b"keep")?;
    let folder = source.join("folder");
    let during = watch_source_during_sync(&folder);
    let _guard = RemovableFlushGuard::install(root.path(), None, false);

    let (_operation, events) = paste_move(
        95,
        &destination,
        &source.join("folder"),
        TransferConflict::Merge,
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
    assert!(!source.join("folder").exists());
    assert_eq!(fs::read(destination.join("folder/incoming.txt"))?, b"new");
    assert_eq!(fs::read(destination.join("folder/stays.txt"))?, b"keep");
    Ok(())
}

#[test]
fn same_filesystem_rename_is_not_held_for_the_removable_flush() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source_dir = root.path().join("source");
    let destination = root.path().join("usb");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    let source = source_dir.join("notes.txt");
    fs::write(&source, b"renamed")?;
    let during = watch_source_during_sync(&source);
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other), false);

    let (_operation, events) =
        paste_move(96, &destination, &source, TransferConflict::FailIfExists);
    pump_until_transfer(&events);
    let seen = during.lock().unwrap_or_else(|error| error.into_inner());
    assert!(
        !seen.is_empty() && seen.iter().all(|exists| !exists),
        "a same-filesystem rename finishes before the removable flush starts: {seen:?}"
    );

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
    assert!(!source.exists());
    Ok(())
}
