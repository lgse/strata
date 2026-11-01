// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT;

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
fn operation_reconciliation_replaces_stale_rows_without_an_empty_publication() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("async test lock");
    for uri_base in [None, Some("trash://")] {
        let (browser, events, source) = scripted_browser(ScriptedSource {
            uri_base,
            ..ScriptedSource::manual(vec![], vec![])
        });
        let root = uri_base.map_or_else(
            || Location::local("/fixture"),
            |_| Location::uri("trash:///"),
        );
        browser.navigate(root);
        let (id, emit) = source.enumerate_calls.borrow()[0].clone();
        emit(DirectoryEvent::Batch {
            request_id: id,
            entries: vec![source.listed_entry("stale", false)],
        });
        emit(DirectoryEvent::Finished {
            request_id: id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        assert_eq!(column_names(&browser, 0), ["stale"]);
        browser.select_all(0);
        events.borrow_mut().clear();

        browser.refresh_operation_columns(&[0]);
        assert_eq!(column_names(&browser, 0), ["stale"]);
        let (id, emit) = source.enumerate_calls.borrow()[1].clone();
        emit(DirectoryEvent::Batch {
            request_id: id,
            entries: vec![source.listed_entry("current", false)],
        });
        assert_eq!(column_names(&browser, 0), ["stale"]);
        emit(DirectoryEvent::Finished {
            request_id: id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        assert_eq!(column_names(&browser, 0), ["current"]);
        assert_eq!(browser.selected_count(), 0);
        assert!(browser.selected_entries().is_empty());
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::EntriesReplaced { depth: 0, count: 1 }))
        );
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            BrowserEvent::ColumnReloaded { .. } | BrowserEvent::EntriesReplaced { count: 0, .. }
        )));
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
    let (browser, events, _) = scripted_browser(ScriptedSource {
        watched: true,
        ..ScriptedSource::scripted(Vec::<&str>::new(), vec![])
    });
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
fn a_rescan_elsewhere_still_drops_what_an_operation_removed_from_an_unwatched_folder() {
    struct Unwatched(Location);

    impl FileSource for Unwatched {
        fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
            Ok(())
        }

        fn enumerate(
            &self,
            request: DirectoryRequest,
            emit: Rc<dyn Fn(DirectoryEvent)>,
        ) -> LoadHandle {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![
                    named(&request.location, "gone.txt", false),
                    named(&request.location, "watched", true),
                ],
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: None,
                can_delete: None,
            });
            LoadHandle::new(|| {})
        }

        fn watch(&self, location: Location, _: bool, _: WatchCallback) -> Option<LoadHandle> {
            (location != self.0).then(|| LoadHandle::new(|| {}))
        }
    }

    type Ending = fn(OperationRequestId, Location) -> OperationEvent;
    let deleted: Ending = |request_id, gone| OperationEvent::Deleted {
        request_id,
        locations: vec![gone],
    };
    let deleted_with_errors: Ending = |request_id, gone| OperationEvent::CompletedWithErrors {
        request_id,
        deleted_locations: vec![gone],
        retryable_locations: Vec::new(),
        has_non_retryable_failures: true,
        message: "one item failed".to_owned(),
    };
    let restored: Ending = |request_id, gone| OperationEvent::Restored {
        request_id,
        locations: vec![gone],
        restored: Vec::new(),
    };
    let restored_with_errors: Ending =
        |request_id, gone| OperationEvent::RestoreCompletedWithErrors {
            request_id,
            restored_locations: vec![gone],
            restored: Vec::new(),
            message: "one item failed".to_owned(),
        };
    let rows = [
        ("deleted", false, deleted),
        ("deleted with errors", false, deleted_with_errors),
        ("restored", true, restored),
        ("restored with errors", true, restored_with_errors),
    ];
    for (row, restoring, ending) in rows {
        let folder = if restoring {
            Location::uri("trash:///")
        } else {
            Location::local("/fixture")
        };
        let watched = child(&folder, "watched");
        let browser = Browser::new(Rc::new(Unwatched(folder.clone())));
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        browser.navigate(folder.clone());
        browser.descend(0, watched.clone());
        let request_id = browser.begin_operation();
        browser.deletion_operation.set(!restoring);
        browser.restoration_operation.set(restoring);
        let emit = browser.operation_callback(request_id, false, HashSet::new());
        browser.handle_directory_change(1, &watched, DirectoryChange::Rescan);

        emit(ending(request_id, child(&folder, "gone.txt")));

        assert_eq!(column_names(&browser, 0), ["watched"], "{row}");
        assert!(
            events.borrow().iter().any(|event| matches!(
                event,
                BrowserEvent::OperationRefreshRequired { depths } if depths == &[1]
            )),
            "{row}"
        );
    }
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
    let outside_focus = Rc::new(std::cell::Cell::new(None));
    let observed_focus = outside_focus.clone();
    browser.observe(move |event| {
        let browser = weak.upgrade().expect("live browser");
        match event {
            BrowserEvent::EntriesSpliced { .. } => {
                assert_eq!(column_names(&browser, 0), ["beta"]);
                assert_eq!(browser.selected_positions(0), [0]);
            }
            BrowserEvent::FocusChanged { .. } => {
                observed_focus.set(Some(browser.focus_follows_external_change()));
            }
            _ => {}
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
                position: Some(0),
                ..
            },
            BrowserEvent::RestorationFinished { succeeded: true },
        ]
    ));
    assert_eq!(
        outside_focus.get(),
        Some(true),
        "deferred monitor changes must not pull focus out of a filter"
    );
}
