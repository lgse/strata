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
fn rescan_discards_incremental_changes_in_the_same_operation_batch() {
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
    assert_eq!(column_names(&browser, 0), ["alpha", "beta"]);
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
            .count(),
        1
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { .. }))
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
            BrowserEvent::RestorationFinished,
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
