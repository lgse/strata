// SPDX-License-Identifier: MIT

use super::*;

/// A `usb` destination and a `photo.bin` source beside it, beneath `root`.
fn photo_for_usb(root: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let destination = root.join("usb");
    fs::create_dir(&destination)?;
    let source = root.join("photo.bin");
    fs::write(&source, b"pixels")?;
    Ok((destination, source))
}

fn copy_request(id: u64, destination: &Path, sources: &[PathBuf]) -> PasteRequest {
    let items = sources
        .iter()
        .map(|source| item(source, TransferConflict::FailIfExists))
        .collect();
    request(id, destination, items, false)
}

/// Pastes, dropping the operation right after the first event `cancel_on`
/// matches is recorded.
fn paste_cancelled_on(
    request: PasteRequest,
    cancel_on: impl Fn(&OperationEvent) -> bool + 'static,
) -> (Rc<RefCell<Option<LoadHandle>>>, Vec<OperationEvent>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let handle = Rc::new(RefCell::new(None));
    let slot = handle.clone();
    let operation = LocalOperationProvider.paste(
        request,
        Rc::new(move |event| {
            let cancel = cancel_on(&event);
            emitted.borrow_mut().push(event);
            if cancel {
                slot.borrow_mut().take();
            }
        }),
    );
    *handle.borrow_mut() = Some(operation);
    pump_until_transfer(&events);
    (handle, events.take())
}

/// The transfer ended as `ended` allows, never as pasted, with the photo
/// written and flushed to the device.
fn assert_photo_flushed_unpasted(
    events: &[OperationEvent],
    ended: fn(Option<&OperationEvent>) -> bool,
    destination: &Path,
) -> Result<(), Box<dyn Error>> {
    assert!(ended(terminal_transfer(events)), "{events:?}");
    let pasted = events
        .iter()
        .any(|event| matches!(event, OperationEvent::Pasted { .. }));
    assert!(!pasted);
    assert_eq!(fs::read(destination.join("photo.bin"))?, b"pixels");
    let flushed = !sync_probe_observations().is_empty();
    assert!(flushed, "written bytes are synced before the transfer ends");
    Ok(())
}

fn cancelled(event: Option<&OperationEvent>) -> bool {
    matches!(event, Some(OperationEvent::Cancelled { .. }))
}

fn failed(event: Option<&OperationEvent>) -> bool {
    matches!(event, Some(OperationEvent::TransferFailed { .. }))
}

fn warns_of_pending_writes(event: Option<&OperationEvent>) -> bool {
    matches!(event, Some(OperationEvent::TransferFailed { message, .. })
        if message.contains("writes may still be pending") && message.contains("wait for safe eject"))
}

/// Stops a transfer into `destination` with nothing done, as a failure
/// reports it, and returns what it emitted.
fn stop_with(
    destination: PathBuf,
    id: u64,
    failure: Option<&str>,
    skip_flush: bool,
) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let emit: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| emitted.borrow_mut().push(event));
    glib::MainContext::default().block_on(stop_transfer(
        &[destination],
        &emit,
        OperationRequestId(id),
        TransferStop {
            completed: Vec::new(),
            failed: Vec::new(),
            not_attempted: Vec::new(),
            affected_locations: HashSet::new(),
            failure: failure.map(str::to_owned),
            skip_flush,
        },
    ));
    events.take()
}

#[test]
fn bytes_already_written_are_flushed_before_cancellation_and_cancel_is_not_pasted()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (destination, source) = photo_for_usb(root.path())?;
    let _guard = RemovableFlushGuard::install(root.path(), None);

    let (_operation, events) =
        paste_cancelled_on(copy_request(99, &destination, &[source]), |event| {
            matches!(
                event,
                OperationEvent::TransferProgress {
                    completed_items, ..
                } if *completed_items >= 1
            )
        });

    assert_photo_flushed_unpasted(&events, cancelled, &destination)
}

#[test]
fn a_cancel_after_the_flush_starts_does_not_report_success() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (destination, source) = photo_for_usb(root.path())?;
    let _guard = RemovableFlushGuard::install(root.path(), None);

    let (_operation, events) =
        paste_cancelled_on(copy_request(100, &destination, &[source]), |event| {
            matches!(event, OperationEvent::FlushingToDevice { .. })
        });

    assert_photo_flushed_unpasted(&events, cancelled, &destination)
}

#[test]
fn bytes_already_written_are_flushed_before_transfer_failure() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (destination, source) = photo_for_usb(root.path())?;
    let _guard = RemovableFlushGuard::install(root.path(), None);

    let missing = root.path().join("missing.bin");
    let (_operation, events) = paste_pumped(copy_request(101, &destination, &[source, missing]));

    assert_photo_flushed_unpasted(&events, failed, &destination)
}

#[test]
fn failed_final_flush_reports_that_writes_may_still_be_pending() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (destination, source) = photo_for_usb(root.path())?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other));

    let (_operation, events) = paste_pumped(copy_request(106, &destination, &[source]));

    assert!(
        warns_of_pending_writes(terminal_transfer(&events)),
        "{events:?}"
    );
    Ok(())
}

#[test]
fn failed_flush_after_cancel_reports_that_writes_may_still_be_pending() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let _guard = RemovableFlushGuard::install(root.path(), Some(io::ErrorKind::Other));

    let events = stop_with(destination, 105, None, false);

    assert!(warns_of_pending_writes(events.last()), "{events:?}");
    assert!(!sync_probe_observations().is_empty());
    Ok(())
}

#[test]
fn fat32_limit_failure_does_not_start_an_uncancellable_device_flush() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("usb");
    fs::create_dir(&destination)?;
    let _guard = RemovableFlushGuard::install(root.path(), None);

    let events = stop_with(
        destination,
        104,
        Some("A file exceeds the FAT32 limit"),
        true,
    );

    assert!(failed(events.last()), "{events:?}");
    let flushing = events
        .iter()
        .any(|event| matches!(event, OperationEvent::FlushingToDevice { .. }));
    assert!(!flushing);
    assert!(sync_probe_observations().is_empty());
    Ok(())
}
