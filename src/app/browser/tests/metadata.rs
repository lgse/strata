// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn size_sort_waits_for_its_metadata_pass() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let browser = Browser::new(Rc::new(SortFillSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
    let observed = events.clone();
    let observed_waker = waker.clone();
    browser.observe(move |event| {
        let finished = matches!(event, BrowserEvent::SortingFinished { .. });
        observed.borrow_mut().push(event.clone());
        if finished && let Some(waker) = observed_waker.borrow_mut().take() {
            waker.wake();
        }
    });

    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    gtk::glib::MainContext::default().block_on(std::future::poll_fn(|cx| {
        let done = events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SortingFinished { .. }));
        if done || std::time::Instant::now() >= deadline {
            std::task::Poll::Ready(())
        } else {
            *waker.borrow_mut() = Some(cx.waker().clone());
            std::task::Poll::Pending
        }
    }));

    let names: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["beta".to_owned(), "alpha".to_owned()]);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::MetadataFilled { .. }) })
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| { matches!(event, BrowserEvent::SortingStarted { .. }) })
            .count(),
        1
    );
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| { matches!(event, BrowserEvent::SortingFinished { .. }) })
            .count(),
        1
    );
}

#[test]
fn multi_chunk_fill_sorts_once_on_its_terminal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Chunks(
            vec![vec![("alpha", 30)], vec![("beta", 10), ("gamma", 20)]],
            MetadataOutcome::Complete,
        )],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 2);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["beta".to_owned(), "gamma".to_owned(), "alpha".to_owned()]
    );
}

#[test]
fn truncated_fill_preserves_the_prior_order() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Chunks(
            vec![vec![("alpha", 30)]],
            MetadataOutcome::Truncated,
        )],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 1);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()]
    );
}

#[test]
fn unsupported_fill_abandons_the_sort_in_order() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::TerminalOnly(MetadataOutcome::Unsupported)],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 1);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
}

#[test]
fn failed_fill_abandons_the_sort_in_order() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Chunks(
            vec![vec![("alpha", 30), ("beta", 10)]],
            MetadataOutcome::Failed,
        )],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(replaced_count(&events), 1);
    assert_eq!(start_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
}

#[test]
fn empty_fill_still_closes_the_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::EmptyComplete],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);

    assert_eq!(start_count(&events), 1);
    assert_eq!(replaced_count(&events), 2);
}

#[test]
fn navigation_cancels_an_awaiting_sort_without_stale_commit() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let published = replaced_count(&events);
    assert_eq!(published, 1);
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    assert_eq!(start_count(&events), 1);

    browser.navigate(Location::local("/elsewhere"));
    assert_eq!(finish_count(&events), 1);
    assert!(browser.sort_awaiting_fill.borrow().is_none());
    assert!(browser.pending_sort.get().is_none());

    let old = source.fill_calls.borrow();
    let old_emit = old[0].emit.clone();
    let old_id = old[0].id;
    drop(old);
    old_emit(DirectoryEvent::MetadataFilled {
        request_id: old_id,
        updates: vec![MetadataUpdate {
            location: Location::local("/fixture/alpha"),
            size: MetadataValue::Known(1),
            modified_unix_seconds: MetadataValue::Known(7),
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        }],
    });
    old_emit(DirectoryEvent::MetadataFinished {
        request_id: old_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), published + 1);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["alpha".to_owned(), "beta".to_owned()]
    );
}

#[test]
fn reload_cancels_an_awaiting_sort_and_ignores_its_terminal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let published = replaced_count(&events);
    assert_eq!(published, 1);
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());

    browser.reload_active();
    assert_eq!(finish_count(&events), 1);
    assert!(browser.sort_awaiting_fill.borrow().is_none());

    let old = source.fill_calls.borrow();
    let old_emit = old[0].emit.clone();
    let old_id = old[0].id;
    drop(old);
    old_emit(DirectoryEvent::MetadataFinished {
        request_id: old_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), published + 1);
    assert_eq!(finish_count(&events), 1);
}

#[test]
fn viewport_flush_never_disturbs_an_active_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never, FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.metadata_pending.borrow_mut().insert(
        0,
        vec![
            ViewportTarget {
                position: 0,
                location: Location::local("/fixture/alpha"),
                include_icon_details: false,
            },
            ViewportTarget {
                position: 1,
                location: Location::local("/fixture/beta"),
                include_icon_details: false,
            },
        ],
    );
    browser.flush_metadata_fills();
    assert_eq!(source.fill_calls.borrow().len(), 1);
    assert!(!source.fill_calls.borrow()[0].full);

    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| source.fill_calls.borrow().len() == 2);
    let calls = source.fill_calls.borrow();
    let viewport_id = calls[0].id;
    let viewport_emit = calls[0].emit.clone();
    let sort_id = calls[1].id;
    let sort_emit = calls[1].emit.clone();
    assert!(calls[1].full);
    assert_ne!(viewport_id, sort_id);
    drop(calls);

    viewport_emit(DirectoryEvent::MetadataFilled {
        request_id: viewport_id,
        updates: vec![MetadataUpdate {
            location: Location::local("/fixture/alpha"),
            size: MetadataValue::Known(30),
            modified_unix_seconds: MetadataValue::Known(7),
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        }],
    });
    viewport_emit(DirectoryEvent::MetadataFinished {
        request_id: viewport_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), 1);
    assert_eq!(finish_count(&events), 0);
    assert!(browser.sort_awaiting_fill.borrow().is_some());

    sort_emit(DirectoryEvent::MetadataFilled {
        request_id: sort_id,
        updates: vec![
            MetadataUpdate {
                location: Location::local("/fixture/alpha"),
                size: MetadataValue::Known(30),
                modified_unix_seconds: MetadataValue::Known(7),
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            },
            MetadataUpdate {
                location: Location::local("/fixture/beta"),
                size: MetadataValue::Known(10),
                modified_unix_seconds: MetadataValue::Known(7),
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            },
        ],
    });
    sort_emit(DirectoryEvent::MetadataFinished {
        request_id: sort_id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(replaced_count(&events), 2);
    assert_eq!(finish_count(&events), 1);
    assert_eq!(
        column_names(&browser, 0),
        vec!["beta".to_owned(), "alpha".to_owned()]
    );
}

#[test]
fn metadata_dispatch_coalesces_on_idle_and_reuses_covered_in_flight_requests() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    let drain_idle = || {
        let done = Rc::new(Cell::new(false));
        let done_for_idle = done.clone();
        gtk::glib::idle_add_local_once(move || done_for_idle.set(true));
        while !done.get() {
            gtk::glib::MainContext::default().iteration(false);
        }
    };
    for _ in 0..3 {
        browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), false);
        browser.request_metadata_fill(0, 1, Location::local("/fixture/beta"), false);
    }
    assert!(source.fill_calls.borrow().is_empty());
    drain_idle();
    assert_eq!(source.fill_calls.borrow().len(), 1);
    assert_eq!(source.fill_calls.borrow()[0].entries.len(), 2);
    assert!(!source.fill_calls.borrow()[0].full);
    for _ in 0..3 {
        browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), false);
        browser.request_metadata_fill(0, 1, Location::local("/fixture/beta"), false);
        drain_idle();
    }
    assert_eq!(
        source.fill_calls.borrow().len(),
        1,
        "in-flight work is not restarted"
    );
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), true);
    drain_idle();
    assert_eq!(source.fill_calls.borrow().len(), 1);
    let (id, emit) = {
        let calls = source.fill_calls.borrow();
        (calls[0].id, calls[0].emit.clone())
    };
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    drain_idle();
    assert_eq!(
        source.fill_calls.borrow().len(),
        2,
        "richer requests reach the source after the active fill finishes"
    );
    assert!(source.fill_calls.borrow()[1].include_icon_details);
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), true);
    drain_idle();
    assert_eq!(source.fill_calls.borrow().len(), 2);
    let (id, emit) = {
        let calls = source.fill_calls.borrow();
        (calls[1].id, calls[1].emit.clone())
    };
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), true);
    drain_idle();
    assert_eq!(
        source.fill_calls.borrow().len(),
        3,
        "completed work releases its claim"
    );
}

#[test]
fn newly_visible_rows_do_not_discard_late_dimensions_for_the_focused_file() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.set_selection(0, &[0], Some(0));
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), true);
    pump_until(|| source.fill_calls.borrow().len() == 1);
    let (id, emit) = {
        let calls = source.fill_calls.borrow();
        (calls[0].id, calls[0].emit.clone())
    };
    let mut update = MetadataUpdate {
        location: Location::local("/fixture/alpha"),
        size: MetadataValue::Known(2048),
        modified_unix_seconds: MetadataValue::Known(7),
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    };
    emit(DirectoryEvent::MetadataFilled {
        request_id: id,
        updates: vec![update.clone()],
    });
    for (position, name) in [(1, "beta"), (2, "gamma")] {
        browser.request_metadata_fill(
            0,
            position,
            Location::local(format!("/fixture/{name}")),
            true,
        );
        pump_until(|| browser.metadata_idle.borrow().is_none());
        assert_eq!(source.fill_calls.borrow().len(), 1);
    }
    events.borrow_mut().clear();
    update.image_dimensions = MetadataValue::Known((1920, 1080));
    emit(DirectoryEvent::MetadataFilled {
        request_id: id,
        updates: vec![update],
    });
    assert_eq!(
        browser
            .entry_at(0, 0)
            .expect("focused file")
            .image_dimensions,
        MetadataValue::Known((1920, 1080))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::MetadataFilled { depth: 0, updates }
            if updates.iter().any(|(position, entry)| *position == 0
                && entry.image_dimensions == MetadataValue::Known((1920, 1080)))
    )));
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    pump_until(|| source.fill_calls.borrow().len() == 2);
    assert_eq!(
        source.fill_calls.borrow()[1].entries,
        vec![
            Location::local("/fixture/beta"),
            Location::local("/fixture/gamma")
        ]
    );
}

#[test]
fn visible_metadata_is_admitted_when_the_offscreen_backlog_is_full() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async lock");
    let (browser, _, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), true);
    pump_until(|| source.fill_calls.borrow().len() == 1);
    let (id, emit) = {
        let calls = source.fill_calls.borrow();
        (calls[0].id, calls[0].emit.clone())
    };
    let mut backlog = (0..MAX_PENDING_FILL_LOCATIONS - 1)
        .map(|index| ViewportTarget {
            position: index + 3,
            location: Location::local(format!("/fixture/offscreen-{index}")),
            include_icon_details: false,
        })
        .collect::<Vec<_>>();
    let beta = Location::local("/fixture/beta");
    let gamma = Location::local("/fixture/gamma");
    backlog.push(ViewportTarget {
        position: 1,
        location: beta.clone(),
        include_icon_details: false,
    });
    browser.metadata_pending.borrow_mut().insert(0, backlog);

    browser.request_visible_metadata_fill(0, 1, beta.clone(), true);
    browser.request_visible_metadata_fill(0, 2, gamma.clone(), false);
    browser.prioritize_metadata_fills(0, &[beta.clone(), gamma.clone()]);
    assert_eq!(
        browser.metadata_pending.borrow()[&0].len(),
        MAX_PENDING_FILL_LOCATIONS
    );
    pump_until(|| browser.metadata_idle.borrow().is_none());
    assert_eq!(
        source.fill_calls.borrow().len(),
        1,
        "active work is not restarted"
    );
    assert!(browser.fill_tokens.borrow().contains_key(&id));
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    pump_until(|| source.fill_calls.borrow().len() == 2);
    let calls = source.fill_calls.borrow();
    assert_eq!(&calls[1].entries[..2], &[beta, gamma]);
    assert!(
        calls[1].include_icon_details,
        "promoted entries retain richer requests"
    );
}

#[test]
fn viewport_metadata_reprioritizes_between_bounded_batches() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async lock");
    let names: Vec<&'static str> = "abcdefghijklmnopqrstuvwxyz"
        .as_bytes()
        .chunks(1)
        .map(|name| std::str::from_utf8(name).expect("ASCII name"))
        .collect();
    let (browser, _, source) = scripted_browser(ScriptedSource::scripted(
        names.clone(),
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    for (position, name) in names.iter().enumerate() {
        browser.request_metadata_fill(
            0,
            position,
            Location::local(format!("/fixture/{name}")),
            true,
        );
    }
    pump_until(|| source.fill_calls.borrow().len() == 1);
    let (id, emit, first_count) = {
        let calls = source.fill_calls.borrow();
        (calls[0].id, calls[0].emit.clone(), calls[0].entries.len())
    };
    assert!(
        first_count < names.len(),
        "offscreen backlog must leave room for reprioritization"
    );
    let visible = Location::local("/fixture/z");
    browser.prioritize_metadata_fills(0, std::slice::from_ref(&visible));
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    pump_until(|| source.fill_calls.borrow().len() == 2);
    let calls = source.fill_calls.borrow();
    assert_eq!(calls[1].entries.first(), Some(&visible));
    let filled: HashSet<_> = calls
        .iter()
        .flat_map(|call| call.entries.iter().cloned())
        .collect();
    assert_eq!(
        filled.len(),
        names.len(),
        "reprioritization retains unfinished files"
    );
}

#[test]
fn shifted_viewport_rows_go_stale_without_repaint() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta", "gamma"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.metadata_pending.borrow_mut().insert(
        0,
        vec![ViewportTarget {
            position: 1,
            location: Location::local("/fixture/beta"),
            include_icon_details: false,
        }],
    );
    browser.flush_metadata_fills();
    assert_eq!(source.fill_calls.borrow().len(), 1);

    browser.state.borrow_mut().columns[0].entries.remove(0);
    let fill = source.fill_calls.borrow();
    let emit = fill[0].emit.clone();
    let id = fill[0].id;
    drop(fill);
    emit(DirectoryEvent::MetadataFilled {
        request_id: id,
        updates: vec![MetadataUpdate {
            location: Location::local("/fixture/beta"),
            size: MetadataValue::Known(10),
            modified_unix_seconds: MetadataValue::Known(7),
            mode: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
        }],
    });
    assert_eq!(replaced_count(&events), 1);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| { matches!(event, BrowserEvent::MetadataFilled { .. }) })
    );
    assert_eq!(
        browser.state.borrow().columns[0].entries[0].size,
        MetadataValue::Unknown
    );
}

#[test]
fn remote_name_listing_fills_visible_metadata() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let mut src = ScriptedSource::scripted(
        vec!["photo.jpg", "notes.txt"],
        vec![FillAnswer::Complete(vec![
            ("photo.jpg", 100),
            ("notes.txt", 50),
        ])],
    );
    src.uri_base = Some("sftp://host/share");
    let (browser, events, source) = scripted_browser(src);
    browser.navigate(Location::uri("sftp://host/share"));
    browser.metadata_pending.borrow_mut().insert(
        0,
        vec![
            ViewportTarget {
                position: 1,
                location: Location::uri("sftp://host/share/photo.jpg"),
                include_icon_details: false,
            },
            ViewportTarget {
                position: 0,
                location: Location::uri("sftp://host/share/notes.txt"),
                include_icon_details: false,
            },
        ],
    );
    browser.flush_metadata_fills();
    assert_eq!(source.fill_calls.borrow().len(), 1);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::MetadataFilled { .. }))
    );
    let sizes: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.size.clone())
        .collect();
    assert_eq!(
        sizes,
        vec![MetadataValue::Known(50), MetadataValue::Known(100)]
    );
}

#[test]
fn modified_sort_fills_directory_mtimes() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let mut src = ScriptedSource::scripted(vec!["b.txt"], vec![FillAnswer::Never]);
    src.dirs = vec!["sub"];
    let (browser, events, source) = scripted_browser(src);
    browser.navigate(Location::local("/fixture"));
    browser.set_sort(0, SortKey::Modified, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());
    let fill = source.fill_calls.borrow();
    assert!(fill[0].full);
    assert!(fill[0].entries.contains(&Location::local("/fixture/sub")));
    let emit = fill[0].emit.clone();
    let id = fill[0].id;
    drop(fill);
    emit(DirectoryEvent::MetadataFilled {
        request_id: id,
        updates: vec![
            MetadataUpdate {
                location: Location::local("/fixture/sub"),
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Known(200),
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            },
            MetadataUpdate {
                location: Location::local("/fixture/b.txt"),
                size: MetadataValue::Known(10),
                modified_unix_seconds: MetadataValue::Known(100),
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            },
        ],
    });
    emit(DirectoryEvent::MetadataFinished {
        request_id: id,
        outcome: MetadataOutcome::Complete,
    });
    assert_eq!(finish_count(&events), 1);
    assert_eq!(replaced_count(&events), 2);
    let column = &browser.state.borrow().columns[0].entries;
    let sub = column
        .iter()
        .find(|entry| entry.location == Location::local("/fixture/sub"))
        .expect("the directory row should still be listed");
    assert_eq!(sub.modified_unix_seconds, MetadataValue::Known(200));
    assert_eq!(sub.size, MetadataValue::Unknown);
}
