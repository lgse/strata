// SPDX-License-Identifier: MIT

use super::*;

/// A restoration reporting `outcome` for `/fixture/report.txt` records it, and only it, for undo.
fn assert_a_restore_records_what_it_restored(
    outcome: impl FnOnce(OperationRequestId, Location) -> OperationEvent,
) {
    let browser = operating_browser();
    let restored = Location::local("/fixture/report.txt");
    let (request_id, emit) = start_operation(&browser, OperationSetup::Restoration);

    emit(outcome(request_id, restored.clone()));

    assert_eq!(pending_undo_entry(), Some(UndoEntry::Copy(vec![restored])));
}

#[test]
fn a_completed_restore_records_the_restored_locations_for_undo() {
    assert_a_restore_records_what_it_restored(|request_id, restored| OperationEvent::Restored {
        request_id,
        locations: vec![Location::uri("trash:///report.txt")],
        restored: vec![restored],
    });
}

#[test]
fn a_partially_completed_restore_records_only_the_restored_locations() {
    assert_a_restore_records_what_it_restored(|request_id, restored| {
        OperationEvent::RestoreCompletedWithErrors {
            request_id,
            restored_locations: vec![Location::uri("trash:///report.txt")],
            restored: vec![restored],
            message: "one item failed".to_owned(),
        }
    });
}

#[test]
fn a_cancelled_restore_records_only_the_completed_restores() {
    assert_a_restore_records_what_it_restored(|request_id, restored| OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: vec![restored],
            failed: Vec::new(),
            not_attempted: vec![Location::local("/fixture/second.txt")],
            affected_locations: HashSet::new(),
        },
    });
}

#[test]
fn undoing_a_trash_delete_does_not_record_the_restore_it_performed() {
    let browser = operating_browser();
    let trashed = Location::local("/fixture/report.txt");
    let (request_id, emit) = start_replay(
        &browser,
        trash_undo(vec![trashed.clone()]),
        OperationSetup::Restoration,
    );

    emit(OperationEvent::Restored {
        request_id,
        locations: Vec::new(),
        restored: vec![trashed],
    });

    assert_eq!(
        pending_undo_entry(),
        None,
        "the restore performed by a trash undo must not record a new entry"
    );
}
