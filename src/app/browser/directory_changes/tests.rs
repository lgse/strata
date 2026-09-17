// SPDX-License-Identifier: MIT

use super::*;

fn finish(emit: &dyn Fn(DirectoryEvent), request_id: RequestId) {
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
}

#[test]
fn staged_moves_and_recreated_entries_override_late_enumeration_rows() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    let old = batch_entry("old");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![old.clone()],
    });
    let recreated = staged_entry("new", EntryKind::File, MetadataValue::Known(42), 10);
    browser.handle_directory_change(
        0,
        &root,
        DirectoryChange::Move {
            from: old.location.clone(),
            entry: batch_entry("new"),
        },
    );
    browser.handle_directory_change(
        0,
        &root,
        DirectoryChange::Remove(recreated.location.clone()),
    );
    browser.handle_directory_change(0, &root, DirectoryChange::Upsert(recreated));
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![old, batch_entry("new")],
    });
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
    );
    finish(emit.as_ref(), request_id);
    assert_eq!(column_names(&browser, 0), ["new"]);
    assert_eq!(
        browser.entry_at(0, 0).expect("recreated entry").size,
        MetadataValue::Known(42)
    );
}

#[test]
fn rescan_restarts_loading_and_rejects_the_superseded_enumeration() {
    let (browser, _, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let (old_id, old_emit) = source.enumerate_calls.borrow()[0].clone();
    old_emit(DirectoryEvent::Batch {
        request_id: old_id,
        entries: vec![batch_entry("old")],
    });
    browser.handle_directory_change(0, &root, DirectoryChange::Rescan);
    let (current_id, current_emit) = source.enumerate_calls.borrow()[1].clone();
    old_emit(DirectoryEvent::Batch {
        request_id: old_id,
        entries: vec![batch_entry("stale")],
    });
    finish(old_emit.as_ref(), old_id);
    assert!(
        browser
            .column_snapshot(0)
            .expect("replacement load")
            .loading
    );
    current_emit(DirectoryEvent::Batch {
        request_id: current_id,
        entries: vec![batch_entry("current")],
    });
    finish(current_emit.as_ref(), current_id);
    assert_eq!(column_names(&browser, 0), ["current"]);
    assert!(
        !browser
            .column_snapshot(0)
            .expect("completed replacement")
            .loading
    );
}

#[test]
fn live_monitor_delta_drains_publication_tails_before_splicing() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async test lock");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: (0..700)
            .map(|index| batch_entry(&format!("file-{index:03}")))
            .collect(),
    });
    finish(emit.as_ref(), request_id);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    browser.handle_directory_change(
        0,
        &root,
        DirectoryChange::Remove(batch_entry("file-699").location),
    );
    assert_eq!(
        browser.column_snapshot(0).expect("loaded column").count,
        699
    );
    let events = events.borrow();
    let finished = events
        .iter()
        .position(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
        .expect("published terminal");
    let spliced = events
        .iter()
        .position(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
        .expect("monitor splice");
    assert!(finished < spliced);
    // Draining leaves an idle armed; remove it on its owning test thread.
    browser.cancel_publish(0);
}

#[test]
fn stale_watchers_cannot_queue_a_rescan_during_file_operations() {
    for restoring in [false, true] {
        let (browser, events, _) =
            scripted_browser(ScriptedSource::scripted(vec!["current"], vec![]));
        browser.navigate(Location::local("/fixture"));
        let request_id = browser.begin_operation();
        browser.deletion_operation.set(!restoring);
        browser.restoration_operation.set(restoring);
        let complete = browser.operation_callback(request_id, false, HashSet::new());
        events.borrow_mut().clear();
        browser.handle_directory_change(
            0,
            &Location::local("/old-location"),
            DirectoryChange::Rescan,
        );
        complete(if restoring {
            OperationEvent::Restored {
                request_id,
                locations: Vec::new(),
                restored: Vec::new(),
            }
        } else {
            OperationEvent::Deleted {
                request_id,
                locations: Vec::new(),
            }
        });
        assert_eq!(column_names(&browser, 0), ["current"]);
        assert!(
            !events
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
        );
    }
}
