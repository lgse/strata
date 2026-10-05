// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn load_finish_applies_rows_queued_behind_the_count_threshold() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });

    let names: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) })
    );
}

#[test]
fn remote_load_finishes_only_after_every_queued_row_is_applied() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("first")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: (0..1025)
            .map(|index| batch_entry(&format!("queued-{index:04}")))
            .collect(),
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    assert_eq!(browser.state.borrow().columns[0].entries.len(), 513);

    browser.flush_coalesced_capped(Some(0));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    browser.flush_coalesced_capped(Some(0));

    let events = events.borrow();
    let finish = events
        .iter()
        .position(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
        .expect("the drained load should finish");
    let last_insert = events
        .iter()
        .rposition(|event| matches!(event, BrowserEvent::EntriesInserted { .. }))
        .expect("the final queued rows should be inserted");
    assert!(finish > last_insert);
    assert_eq!(browser.state.borrow().columns[0].entries.len(), 1026);
}

#[test]
fn remote_load_failure_waits_for_queued_rows() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("sftp://host/fixture"));
    let (request_id, emit) = captured
        .borrow()
        .clone()
        .expect("navigate should start a directory load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("first")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: (0..513)
            .map(|index| batch_entry(&format!("queued-{index:04}")))
            .collect(),
    });
    emit(DirectoryEvent::Failed {
        request_id,
        message: "remote failure".to_owned(),
    });

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFailed { .. }))
    );
    browser.flush_coalesced_capped(Some(0));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LoadFailed { message, .. } if message == "remote failure"
    )));
    assert_eq!(browser.state.borrow().columns[0].entries.len(), 514);
}

#[test]
fn refresh_drops_staging_and_its_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    assert_eq!(
        browser
            .staging
            .borrow()
            .get(&0)
            .map(|staged| staged.entries.len()),
        Some(2)
    );
    assert_eq!(replaced_count(&events), 0);

    browser.refresh_column(0);
    assert!(!browser.staging.borrow().contains_key(&0));
    assert_eq!(replaced_count(&events), 0);

    let (request_id, emit) = source.enumerate_calls.borrow()[1].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    assert_eq!(replaced_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );

    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    assert!(browser.sort_awaiting_fill.borrow().is_some());
    browser.refresh_column(0);
    assert!(browser.sort_awaiting_fill.borrow().is_none());
    assert!(browser.pending_sort.get().is_none());
    assert_eq!(start_count(&events), 1);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(replaced_count(&events), 1);
}

#[test]
fn close_column_clears_the_truncated_depth() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });

    browser.descend(0, Location::local("/fixture/sub"));
    let (sub_id, sub_emit) = source.enumerate_calls.borrow()[1].clone();
    sub_emit(DirectoryEvent::Batch {
        request_id: sub_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    sub_emit(DirectoryEvent::Finished {
        request_id: sub_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    let published = replaced_count(&events);
    assert_eq!(published, 2);

    browser.set_sort(1, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    assert!(browser.sort_awaiting_fill.borrow().is_some());

    browser.descend(1, Location::local("/fixture/sub/deep"));
    let (deep_id, deep_emit) = source.enumerate_calls.borrow()[2].clone();
    deep_emit(DirectoryEvent::Batch {
        request_id: deep_id,
        entries: vec![batch_entry("alpha")],
    });
    assert!(browser.staging.borrow().contains_key(&2));

    browser.close_column(1);
    assert!(!browser.staging.borrow().contains_key(&2));
    assert!(!browser.metadata_pending.borrow().contains_key(&1));
    assert!(!browser.sort_loads.borrow().contains_key(&1));
    assert!(browser.sort_awaiting_fill.borrow().is_none());
    assert!(browser.pending_sort.get().is_none());
    assert_eq!(start_count(&events), 1);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(replaced_count(&events), published);
}

#[test]
fn native_initial_load_publishes_sorted_once() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("gamma"), batch_entry("alpha")],
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    assert!(browser.staging.borrow().contains_key(&0));
    assert!(
        !events.borrow().iter().any(|event| {
            matches!(
                event,
                BrowserEvent::EntriesInserted { .. }
                    | BrowserEvent::EntriesPublished { .. }
                    | BrowserEvent::EntriesReplaced { .. }
            )
        }),
        "staging must not publish provisional rows"
    );
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    assert_eq!(replaced_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()]
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
    assert!(browser.staging.borrow().is_empty());
}

#[test]
fn empty_native_initial_load_finishes_without_a_batch() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();

    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });

    assert_eq!(replaced_count(&events), 1);
    assert!(column_names(&browser, 0).is_empty());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LoadFinished {
            depth: 0,
            truncated: false
        }
    )));
}

#[test]
fn incomplete_native_metadata_uses_name_order_until_a_full_retry_finishes() {
    let source = Rc::new(ScriptedSource::manual(vec![], vec![FillAnswer::Never]));
    let browser = Browser::with_preferences(
        source.clone(),
        ViewPreferences {
            sort_key: SortKey::Size,
            ..ViewPreferences::default()
        },
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![
            staged_entry("beta", EntryKind::File, MetadataValue::Known(1), 1),
            staged_entry("alpha", EntryKind::File, MetadataValue::Unknown, 1),
        ],
    });
    emit(DirectoryEvent::MetadataIncomplete { request_id });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });

    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
    assert_eq!(start_count(&events), 1);
    assert!(browser.sort_awaiting_fill.borrow().is_some());
    let fills = source.fill_calls.borrow();
    assert_eq!(fills.len(), 1);
    assert!(fills[0].full);
    assert_ne!(fills[0].id, request_id);
}

#[test]
fn staged_load_reconciles_monitor_deltas_without_resurrection() {
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha"), batch_entry("beta")],
    });
    browser.handle_directory_change(
        0,
        &Location::local("/fixture"),
        DirectoryChange::Remove(Location::local("/fixture/beta")),
    );
    browser.handle_directory_change(
        0,
        &Location::local("/fixture"),
        DirectoryChange::Upsert(batch_entry("gamma")),
    );
    assert!(
        !events.borrow().iter().any(|event| {
            matches!(
                event,
                BrowserEvent::EntriesSpliced { .. }
                    | BrowserEvent::EntriesPublished { .. }
                    | BrowserEvent::EntriesReplaced { .. }
            )
        }),
        "queued deltas must not touch the UI before the reconcile"
    );
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "gamma".to_owned()]
    );
}

#[test]
fn staged_sorts_order_every_key_in_both_directions() {
    use crate::model::{SortDirection, SortKey, ViewPreferences};
    let entries = || {
        vec![
            staged_entry("b.txt", EntryKind::File, MetadataValue::Known(20), 100),
            staged_entry("sub", EntryKind::Directory, MetadataValue::Unknown, 150),
            staged_entry("a.txt", EntryKind::File, MetadataValue::Known(10), 200),
        ]
    };
    let cases: &[(SortKey, SortDirection, bool, [&str; 3])] = &[
        (
            SortKey::Name,
            SortDirection::Ascending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Name,
            SortDirection::Ascending,
            false,
            ["a.txt", "b.txt", "sub"],
        ),
        (
            SortKey::Name,
            SortDirection::Descending,
            true,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Name,
            SortDirection::Descending,
            false,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Ascending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Ascending,
            false,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Descending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Type,
            SortDirection::Descending,
            false,
            ["a.txt", "b.txt", "sub"],
        ),
        (
            SortKey::Size,
            SortDirection::Ascending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Size,
            SortDirection::Ascending,
            false,
            ["a.txt", "b.txt", "sub"],
        ),
        (
            SortKey::Size,
            SortDirection::Descending,
            true,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Size,
            SortDirection::Descending,
            false,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Ascending,
            true,
            ["sub", "b.txt", "a.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Ascending,
            false,
            ["b.txt", "sub", "a.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Descending,
            true,
            ["sub", "a.txt", "b.txt"],
        ),
        (
            SortKey::Modified,
            SortDirection::Descending,
            false,
            ["a.txt", "sub", "b.txt"],
        ),
    ];
    for (key, direction, folders_first, expected) in cases {
        let source: Rc<ScriptedSource> = Rc::new(ScriptedSource::manual(vec![], vec![]));
        let browser = Browser::with_preferences(
            source.clone(),
            ViewPreferences {
                sort_key: *key,
                sort_direction: *direction,
                folders_first: *folders_first,
                ..ViewPreferences::default()
            },
        );
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));
        browser.navigate(Location::local("/fixture"));
        let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
        emit(DirectoryEvent::Batch {
            request_id,
            entries: entries(),
        });
        emit(DirectoryEvent::Finished {
            request_id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        assert_eq!(
            column_names(&browser, 0),
            expected
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            "key {key:?} direction {direction:?} folders_first {folders_first}"
        );
        assert_eq!(replaced_count(&events), 1);
    }
}

#[test]
fn large_load_streams_prefix_then_tails_with_terminal_last() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::local("/fixture"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    let first: Vec<FileEntry> = (0..400)
        .rev()
        .map(|index| batch_entry(&format!("file-{index:03}")))
        .collect();
    let second: Vec<FileEntry> = (400..700)
        .rev()
        .map(|index| batch_entry(&format!("file-{index:03}")))
        .collect();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: first,
    });
    emit(DirectoryEvent::Batch {
        request_id,
        entries: second,
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    pump_until(|| {
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    });
    let kinds: Vec<(usize, usize)> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::EntriesReplaced { count, .. } => Some((0, *count)),
            BrowserEvent::EntriesPublished { count, .. } => Some((1, *count)),
            BrowserEvent::EntriesInserted { insertions, .. } => Some((
                1,
                insertions
                    .iter()
                    .map(|insertion| insertion.entries.len())
                    .sum(),
            )),
            BrowserEvent::LoadFinished { .. } => Some((2, 0)),
            _ => None,
        })
        .collect();
    assert!(!kinds.is_empty());
    assert_eq!(kinds[0], (0, 128));
    assert_eq!(
        *kinds.last().expect("a terminal should close the stream"),
        (2, 0)
    );
    let streamed: usize = kinds.iter().map(|(_, count)| count).sum();
    assert_eq!(streamed, 700);
    let names = column_names(&browser, 0);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert!(browser.staged_publishes.borrow().is_empty());
}

#[test]
fn remote_rows_flush_within_the_latency_bound() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::manual(vec![], vec![]));
    browser.navigate(Location::uri("sftp://host/share"));
    let (request_id, emit) = source.enumerate_calls.borrow()[0].clone();
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("alpha")],
    });
    assert_eq!(column_names(&browser, 0), vec!["alpha".to_owned()]);
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![batch_entry("beta")],
    });
    assert_eq!(column_names(&browser, 0), vec!["alpha".to_owned()]);
    let start = std::time::Instant::now();
    while column_names(&browser, 0).len() < 2 && start.elapsed() < std::time::Duration::from_secs(5)
    {
        gtk::glib::MainContext::default().iteration(true);
    }
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) }),
        "the load is still open; only the latency flush fired"
    );
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) })
    );
}

#[test]
fn resort_after_mid_load_preference_change_republishes() {
    let (browser, events, _) =
        scripted_browser(ScriptedSource::scripted(vec!["b.txt", "a.txt"], vec![]));
    browser.navigate(Location::local("/fixture"));
    assert_eq!(
        column_names(&browser, 0),
        vec!["a.txt".to_owned(), "b.txt".to_owned()]
    );
    let request_id = browser
        .state
        .borrow()
        .request_id_for_depth(0)
        .expect("the load should still own its request");
    browser.sorting.borrow_mut().insert(
        0,
        SortingLoad {
            request_id,
            deltas: Vec::new(),
        },
    );
    let sorted = vec![batch_entry("b.txt"), batch_entry("a.txt")];
    let staged_preferences = ViewPreferences {
        sort_key: crate::model::SortKey::Size,
        ..ViewPreferences::default()
    };
    browser.finish_staged_sort(
        sorting::SortTask {
            depth: 0,
            request_id,
            plan: sorting::SortPlan {
                ordering_preferences: staged_preferences,
                staged_preferences,
                retry_metadata: false,
                completion: loading::LoadCompletion {
                    truncated: false,
                    can_trash: None,
                    can_delete: None,
                },
            },
        },
        sorted,
    );
    assert_eq!(
        column_names(&browser, 0),
        vec!["a.txt".to_owned(), "b.txt".to_owned()]
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::LoadFinished { .. }) })
    );
}
