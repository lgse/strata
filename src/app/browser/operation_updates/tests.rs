// SPDX-License-Identifier: MIT

use super::*;

fn queue_until_terminal(browser: &Rc<Browser>, restoring: bool) -> impl FnOnce() {
    let request_id = browser.begin_operation();
    browser.deletion_operation.set(!restoring);
    browser.restoration_operation.set(restoring);
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    move || {
        callback(if restoring {
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
    }
}

#[test]
fn bulk_transfer_progress_keeps_completed_files_visible_without_reloading() {
    let (browser, events, _) =
        scripted_browser(ScriptedSource::scripted(Vec::<&str>::new(), vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser.transfer_destination.replace(Some(root.clone()));
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    events.borrow_mut().clear();

    let completed = (0..OPERATION_PUBLICATION_BATCH)
        .map(|index| batch_entry(&format!("file-{index:03}")))
        .collect::<Vec<_>>();
    for entry in &completed {
        browser.handle_directory_change(0, &root, DirectoryChange::Upsert(entry.clone()));
    }
    callback(OperationEvent::TransferProgress {
        request_id,
        completed_items: completed.len(),
        completed_files: completed.len(),
        total_files: Some(completed.len()),
        current_file: None,
        transferred_bytes: 0,
        total_bytes: Some(0),
        created_location: None,
    });

    let snapshot = browser.column_snapshot(0).expect("destination column");
    assert_eq!(snapshot.count, completed.len());
    assert!(!snapshot.loading);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );

    callback(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: completed
                .iter()
                .map(|entry| entry.location.clone())
                .collect(),
            failed: Vec::new(),
            not_attempted: Vec::new(),
            affected_locations: HashSet::from([root]),
        },
    });
    let snapshot = browser.column_snapshot(0).expect("destination column");
    assert_eq!(snapshot.count, completed.len());
    assert!(!snapshot.loading);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

#[test]
fn bulk_trash_and_restore_update_the_listing_without_loading_gaps() {
    let names = (0..OPERATION_PUBLICATION_BATCH)
        .map(|index| format!("file-{index:03}"))
        .collect::<Vec<_>>();
    let (browser, events, _) =
        scripted_browser(ScriptedSource::scripted(Vec::<&str>::new(), vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let entries = names
        .iter()
        .map(|name| batch_entry(name))
        .collect::<Vec<_>>();
    let locations = entries
        .iter()
        .map(|entry| entry.location.clone())
        .collect::<Vec<_>>();
    for entry in &entries {
        browser.handle_directory_change(0, &root, DirectoryChange::Upsert(entry.clone()));
    }
    assert_eq!(
        browser.column_snapshot(0).expect("loaded column").count,
        OPERATION_PUBLICATION_BATCH
    );

    let delete_id = browser.begin_operation();
    browser.deletion_operation.set(true);
    let delete = browser.operation_callback(delete_id, false, HashSet::new());
    events.borrow_mut().clear();
    for location in &locations {
        browser.handle_directory_change(0, &root, DirectoryChange::Remove(location.clone()));
    }
    delete(OperationEvent::DeleteProgress {
        request_id: delete_id,
        completed: locations.len(),
        total: locations.len(),
        deleted_locations: locations.clone(),
    });
    let snapshot = browser.column_snapshot(0).expect("trash source column");
    assert_eq!(snapshot.count, 0);
    assert!(!snapshot.loading);
    delete(OperationEvent::Deleted {
        request_id: delete_id,
        locations: locations.clone(),
    });

    let restore_id = browser.begin_operation();
    browser.restoration_operation.set(true);
    let restore = browser.operation_callback(restore_id, false, HashSet::new());
    for entry in &entries {
        browser.handle_directory_change(0, &root, DirectoryChange::Upsert(entry.clone()));
    }
    restore(OperationEvent::RestoreProgress {
        request_id: restore_id,
        completed: entries.len(),
        total: entries.len(),
        restored_location: locations.last().cloned(),
    });
    let snapshot = browser
        .column_snapshot(0)
        .expect("restored destination column");
    assert_eq!(snapshot.count, entries.len());
    assert!(!snapshot.loading);
    restore(OperationEvent::Restored {
        request_id: restore_id,
        locations: locations.clone(),
        restored: locations,
    });

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

#[test]
fn operation_rescan_defers_one_reconciliation_without_discarding_incremental_changes() {
    let (browser, events, _) =
        scripted_browser(ScriptedSource::scripted(Vec::<&str>::new(), vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let request_id = browser.begin_operation();
    browser.transfer_operation.set(Some(false));
    browser.transfer_destination.replace(Some(root.clone()));
    let callback = browser.operation_callback(request_id, false, HashSet::new());
    events.borrow_mut().clear();

    for index in 0..OPERATION_PUBLICATION_BATCH {
        browser.handle_directory_change(
            0,
            &root,
            DirectoryChange::Upsert(batch_entry(&format!("file-{index:03}"))),
        );
    }
    browser.handle_directory_change(0, &root, DirectoryChange::Rescan);
    callback(OperationEvent::TransferProgress {
        request_id,
        completed_items: OPERATION_PUBLICATION_BATCH,
        completed_files: OPERATION_PUBLICATION_BATCH,
        total_files: Some(OPERATION_PUBLICATION_BATCH),
        current_file: None,
        transferred_bytes: 0,
        total_bytes: Some(0),
        created_location: None,
    });

    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("destination column")
            .count,
        OPERATION_PUBLICATION_BATCH
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );

    callback(OperationEvent::Cancelled {
        request_id,
        result: CancelledOperation {
            completed: Vec::new(),
            failed: Vec::new(),
            not_attempted: Vec::new(),
            affected_locations: HashSet::from([root]),
        },
    });

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationRefreshRequired { depths } if depths == &[0]
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn rescan_applies_incremental_changes_before_requesting_reconciliation() {
    let (browser, events, _) =
        scripted_browser(ScriptedSource::scripted(vec!["alpha", "beta"], vec![]));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    let complete = queue_until_terminal(&browser, false);
    events.borrow_mut().clear();
    browser.handle_directory_change(
        0,
        &root,
        DirectoryChange::Remove(batch_entry("alpha").location),
    );
    browser.handle_directory_change(0, &root, DirectoryChange::Rescan);
    complete();
    assert_eq!(column_names(&browser, 0), ["beta"]);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OperationRefreshRequired { depths } if depths == &[0]
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn queued_parent_move_rebases_descendants_before_their_stale_batches() {
    let (browser, source) = tree(false);
    let complete = queue_until_terminal(&browser, false);
    let old_parent = child(&source.root, "old");
    let old_nested = child(&old_parent, "nested");
    browser.handle_directory_change(
        2,
        &old_nested,
        DirectoryChange::Remove(child(&old_nested, "leaf.txt")),
    );
    let moved = named(&source.root, "renamed", true);
    browser.handle_directory_change(
        0,
        &source.root,
        DirectoryChange::Move {
            from: old_parent,
            entry: moved.clone(),
        },
    );
    source.renamed.set(true);
    complete();
    let new_nested = child(&moved.location, "nested");
    assert_eq!(browser.location_at(2), Some(new_nested.clone()));
    assert_eq!(
        browser.selected_entries()[0].location,
        child(&new_nested, "leaf.txt")
    );
    assert!(source.watches.borrow().contains(&new_nested));
    assert!(source.cancelled_watches.borrow().contains(&old_nested));
}

#[test]
fn queued_ancestor_removal_restores_the_surviving_path_before_child_batches() {
    for restoring in [false, true] {
        let (browser, source) = tree(false);
        let complete = queue_until_terminal(&browser, restoring);
        let old = child(&source.root, "old");
        browser.handle_directory_change(1, &old, DirectoryChange::Remove(child(&old, "nested")));
        browser.handle_directory_change(0, &source.root, DirectoryChange::Remove(old));
        complete();
        assert_eq!(browser.location_at(0), Some(source.root.clone()));
        assert!(browser.location_at(1).is_none());
        assert_eq!(browser.active_depth(), Some(0));
    }
}

#[test]
fn incremental_batch_publishes_final_selection_without_holding_state_borrows() {
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![],
    ));
    let root = Location::local("/fixture");
    browser.navigate(root.clone());
    browser.select(0, 1);
    let complete = queue_until_terminal(&browser, true);
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::EntriesSpliced { .. }) {
            let browser = weak.upgrade().expect("live browser");
            assert_eq!(column_names(&browser, 0), ["beta"]);
            assert_eq!(browser.selected_positions(0), [0]);
        }
    });
    events.borrow_mut().clear();
    for name in ["alpha", "gamma"] {
        browser.handle_directory_change(
            0,
            &root,
            DirectoryChange::Remove(batch_entry(name).location),
        );
    }
    complete();
    let events = events.borrow();
    assert!(matches!(
        events.as_slice(),
        [
            BrowserEvent::EntriesSpliced { depth: 0, .. },
            BrowserEvent::SelectionSetChanged {
                depth: 0,
                focused: 0,
                take_focus: false,
                ..
            },
            BrowserEvent::FocusChanged {
                depth: 0,
                position: Some(0)
            },
            BrowserEvent::RestorationFinished { succeeded: true },
        ]
    ));
}

#[test]
fn preferred_refresh_includes_columns_without_monitor_batches_but_empty_work_is_silent() {
    let (browser, source) = tree(false);
    let requests: Vec<_> = (0..3)
        .map(|depth| browser.column_request_id(depth))
        .collect();
    assert!(!browser.flush_deferred_file_operation_changes(HashMap::new(), true));
    assert_eq!(source.loads.borrow().len(), 3);
    let changes = HashMap::from([(
        0,
        vec![(
            source.root.clone(),
            DirectoryChange::Remove(child(&source.root, "sibling.txt")),
        )],
    )]);
    assert!(browser.flush_deferred_file_operation_changes(changes, true));
    for (depth, request) in requests.into_iter().enumerate() {
        assert_ne!(browser.column_request_id(depth), request);
    }
    assert_eq!(source.loads.borrow().len(), 6);
}
