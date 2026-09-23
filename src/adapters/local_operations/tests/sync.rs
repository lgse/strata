// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::super::{MountFlushHint, SyncDestination, removable_sync_roots, sync_filesystem};
use super::*;

fn hint(root: &str, removable: bool) -> MountFlushHint {
    MountFlushHint {
        root: PathBuf::from(root),
        removable,
    }
}

#[test]
fn removable_copy_roots_are_synced_once() {
    let hints = [
        hint("/media/usb", true),
        hint("/home", false),
        hint("/", false),
    ];

    let removable = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/media/usb/photos/a.jpg"))],
        &hints,
    );
    assert_eq!(removable, vec![PathBuf::from("/media/usb")]);

    let internal = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/home/user/a.jpg"))],
        &hints,
    );
    assert!(internal.is_empty());

    let once = removable_sync_roots(
        &[
            SyncDestination::Local(Path::new("/media/usb/a.jpg")),
            SyncDestination::Local(Path::new("/media/usb/nested/b.jpg")),
        ],
        &hints,
    );
    assert_eq!(once, vec![PathBuf::from("/media/usb")]);

    let unknown = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/var/tmp/file"))],
        &[hint("/media/usb", true)],
    );
    assert!(unknown.is_empty());

    let remote = removable_sync_roots(&[SyncDestination::NonLocal], &hints);
    assert!(remote.is_empty());
}

#[test]
fn sync_filesystem_on_a_local_directory_succeeds() {
    let directory = tempfile::tempdir().expect("temp directory");
    sync_filesystem(directory.path()).expect("syncfs on a local directory");
}

#[test]
fn bytes_already_written_are_flushed_before_cancellation_and_cancel_is_not_pasted()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), None, false);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = Rc::new(RefCell::new(None));
    let slot = handle.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(99),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| {
            let cancel = matches!(
                &event,
                OperationEvent::TransferProgress {
                    completed_items, ..
                } if *completed_items >= 1
            );
            emitted.borrow_mut().push(event);
            if cancel {
                slot.borrow_mut().take();
            }
        }),
    );
    *handle.borrow_mut() = Some(operation);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    assert!(
        !sync_probe_observations().is_empty(),
        "written bytes are synced before cancellation is reported"
    );
    Ok(())
}

#[test]
fn a_cancel_after_the_flush_starts_does_not_report_success() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), None, false);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = Rc::new(RefCell::new(None));
    let slot = handle.clone();
    let operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(100),
            destination: Location::local(&destination),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::FailIfExists,
            }],
            move_sources: false,
        },
        Rc::new(move |event| {
            let cancel = matches!(event, OperationEvent::FlushingToDevice { .. });
            emitted.borrow_mut().push(event);
            if cancel {
                slot.borrow_mut().take();
            }
        }),
    );
    *handle.borrow_mut() = Some(operation);
    pump_until_transfer(&events);

    assert!(matches!(
        terminal_transfer(&events.borrow()),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Pasted { .. }))
    );
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}

#[test]
fn bytes_already_written_are_flushed_before_transfer_failure() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let source = root.path().join("photo.bin");
    fs::write(&source, b"pixels")?;
    let _guard = RemovableFlushGuard::install(root.path(), None, false);
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(101),
            destination: Location::local(&destination),
            items: vec![
                PasteItem {
                    source: Location::local(&source),
                    conflict: TransferConflict::FailIfExists,
                },
                PasteItem {
                    source: Location::local(root.path().join("missing.bin")),
                    conflict: TransferConflict::FailIfExists,
                },
            ],
            move_sources: false,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
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
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}
