// SPDX-License-Identifier: MIT

mod fixed_names;
mod moves;
mod renames;
mod replacements;

use rustix::io::Errno;

use super::fs_quirks::{Op, Preset, Racer, Support, skips_malformed_name_on_fuse};
use super::*;
use crate::services::{
    CancelledOperation, KeptCause, KeptItem, RenameCause, RenameRequest, RenamedItem,
};

fn inode(path: &Path) -> u64 {
    std::os::unix::fs::MetadataExt::ino(&fs::symlink_metadata(path).expect("entry exists"))
}

fn record(original: &Path, current: &Path) -> MoveRecord {
    MoveRecord {
        original: Location::local(original),
        current: Location::local(current),
        renamed: Vec::new(),
        replaced: None,
    }
}

fn fail_if_exists(record: MoveRecord) -> UndoMoveItem {
    UndoMoveItem {
        record,
        conflict: TransferConflict::FailIfExists,
    }
}

/// An undo that keeps names as they were and cleans nothing up.
fn undo_request(id: u64, items: Vec<UndoMoveItem>) -> UndoMoveRequest {
    UndoMoveRequest {
        names: NameMode::Keep,
        cleanup_locations: Vec::new(),
        id: OperationRequestId(id),
        items,
    }
}

fn start_undo_move(request: UndoMoveRequest) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    start_undo_move_observing(request, |_| {})
}

fn start_undo_move_observing(
    request: UndoMoveRequest,
    observe: impl Fn(&OperationEvent) + 'static,
) -> (LoadHandle, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.undo_move(
        request,
        Rc::new(move |event| {
            observe(&event);
            emitted.borrow_mut().push(event);
        }),
    );
    (operation, events)
}

/// Undoes the moves and returns every event up to the transfer's end.
fn undo_move(request: UndoMoveRequest) -> Vec<OperationEvent> {
    let (_operation, events) = start_undo_move(request);
    drive_until_transfer_settles(&events);
    events.take()
}

fn undo_rename(id: u64, current: &Path, original: &Path) -> Vec<OperationEvent> {
    let request = UndoRenameRequest {
        id: OperationRequestId(id),
        current: Location::local(current),
        original: Location::local(original),
    };
    settle(|emit| LocalOperationProvider.undo_rename(request, emit))
}

fn failure(events: &[OperationEvent]) -> String {
    match events {
        [OperationEvent::Failed { message, .. }] => message.clone(),
        other => panic!("expected one failure, got {other:?}"),
    }
}

fn reported_renames(events: &[OperationEvent]) -> Vec<RenamedItem> {
    events
        .iter()
        .flat_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => report.renamed.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn completed_before_failing(events: &[OperationEvent]) -> &[Location] {
    match events.last() {
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) => completed_locations,
        other => panic!("expected a transfer failure, got {other:?}"),
    }
}

fn assert_pasted(events: &[OperationEvent], row: &str) {
    assert!(
        matches!(events.last(), Some(OperationEvent::Pasted { .. })),
        "{row}: {events:?}"
    );
}

fn assert_renamed(events: &[OperationEvent]) {
    assert!(
        matches!(events.last(), Some(OperationEvent::Renamed { .. })),
        "{events:?}"
    );
}

fn cancelled_result<'a>(events: &'a [OperationEvent], row: &str) -> &'a CancelledOperation {
    let Some(OperationEvent::Cancelled { result, .. }) = events.last() else {
        panic!("{row}: expected a cancel, got {events:?}");
    };
    result
}

/// The items a cancel completed, failed and did not attempt.
fn outcome(result: &CancelledOperation) -> (Vec<Location>, Vec<Location>, Vec<Location>) {
    (
        result.completed.clone(),
        result.failed.clone(),
        result.not_attempted.clone(),
    )
}

fn expect_contents(expected: &[(PathBuf, &[u8])]) {
    for (path, contents) in expected {
        let read = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(read, *contents, "{}", path.display());
    }
}

fn expect_names(expected: &[(&Path, &[&str])]) -> io::Result<()> {
    for (folder, names) in expected {
        assert_eq!(super::names(folder)?, *names, "{}", folder.display());
    }
    Ok(())
}

/// Asserts that nothing is at the paths, following symlinks as `exists` does.
fn expect_gone(paths: &[&Path]) {
    for path in paths {
        assert!(!path.exists(), "{}", path.display());
    }
}
