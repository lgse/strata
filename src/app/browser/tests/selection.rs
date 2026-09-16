// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn selecting_entries_by_name_preserves_the_full_matching_selection() {
    let browser = Browser::new(Rc::new(RestoredSortingSource));
    browser.navigate(Location::local("/fixture"));

    browser.select_entries_by_name(&["small".to_owned(), "large".to_owned()]);

    let snapshot = browser.column_snapshot(0).expect("initial column");
    let selected_names: Vec<_> = snapshot
        .selected_positions
        .iter()
        .map(|&position| {
            browser
                .entry_at(0, position)
                .expect("selected positions should resolve")
                .display_name
        })
        .collect();
    assert_eq!(selected_names, ["large", "small"]);
}

#[test]
fn selecting_named_entries_reveals_only_requested_hidden_matches() {
    for by_location in [false, true] {
        for names in [
            vec![".secret.txt"],
            vec!["visible.txt", ".secret.txt"],
            vec!["visible.txt"],
            vec!["missing.txt"],
            vec![],
        ] {
            let source = ScriptedSource::scripted(vec!["visible.txt", ".secret.txt"], Vec::new());
            let browser = Browser::new(Rc::new(source));
            browser.navigate(Location::local("/fixture"));
            let events = Rc::new(RefCell::new(Vec::new()));
            let observed = events.clone();
            browser.observe(move |event| observed.borrow_mut().push(event.clone()));

            let found = if by_location {
                let locations: Vec<_> = names
                    .iter()
                    .map(|name| Location::local(format!("/fixture/{name}")))
                    .collect();
                browser.select_entries_by_location_at(0, &locations)
            } else {
                let names: Vec<_> = names.iter().map(|name| (*name).to_owned()).collect();
                browser.select_entries_by_name_at(0, &names)
            };

            let reveals_hidden = names.contains(&".secret.txt");
            assert_eq!(browser.preferences().show_hidden, reveals_hidden);
            assert_eq!(found, !names.is_empty() && !names.contains(&"missing.txt"));
            if found {
                let mut selected: Vec<_> = browser
                    .selected_positions(0)
                    .into_iter()
                    .map(|position| {
                        browser
                            .entry_at(0, position)
                            .expect("selected entry")
                            .display_name
                    })
                    .collect();
                selected.sort();
                let mut expected = names.clone();
                expected.sort();
                assert_eq!(selected, expected);
            }
            let events = events.borrow();
            let hidden_event = events.iter().position(|event| {
                matches!(event, BrowserEvent::HiddenToggled { show_hidden: true })
            });
            assert_eq!(hidden_event.is_some(), reveals_hidden);
            if let Some(hidden_event) = hidden_event {
                let selection_event = events
                    .iter()
                    .position(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
                    .expect("selection event");
                assert!(hidden_event < selection_event);
            }
        }
    }
}

#[test]
fn reload_active_preserves_a_multi_selection() {
    let browser = Browser::new(Rc::new(RestoredSortingSource));
    browser.navigate(Location::local("/fixture"));
    browser.set_selection(0, &[0, 1], Some(1));
    assert_eq!(browser.selected_positions(0), [0, 1]);

    browser.reload_active();

    assert_eq!(browser.selected_positions(0), [0, 1]);
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("reloaded column")
            .selected_positions,
        vec![0, 1]
    );
}

#[test]
fn removals_preserve_neighbor_selection_without_refocusing_unrelated_entries() {
    for (names, focused, removed, expected, focus_changed) in [
        (vec!["alpha", "bravo", "charlie"], 1, "bravo", Some(1), true),
        (
            vec!["alpha", "bravo", "charlie"],
            2,
            "charlie",
            Some(1),
            true,
        ),
        (vec!["alpha"], 0, "alpha", None, true),
        (
            vec!["alpha", "bravo", "charlie"],
            1,
            "alpha",
            Some(0),
            false,
        ),
        (
            vec!["alpha", "bravo", "charlie"],
            0,
            "charlie",
            Some(0),
            false,
        ),
    ] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        let parent = Location::local("/fixture");
        browser.navigate(parent.clone());
        browser.handle_directory_change(
            0,
            &parent,
            DirectoryChange::Remove(Location::local("/fixture/child")),
        );
        for name in names {
            browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry(name)));
        }
        browser.select(0, focused);
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| observed.borrow_mut().push(event.clone()));

        browser.handle_directory_change(
            0,
            &parent,
            DirectoryChange::Remove(Location::local(format!("/fixture/{removed}"))),
        );

        assert_eq!(
            browser.selected_positions(0),
            expected.into_iter().collect::<Vec<_>>()
        );
        assert_eq!(
            browser.focused_item().map(|(_, position, _)| position),
            expected
        );
        let notifications: Vec<_> = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                BrowserEvent::FocusChanged { depth: 0, position } => Some(*position),
                _ => None,
            })
            .collect();
        assert_eq!(
            notifications,
            if focus_changed {
                vec![expected]
            } else {
                vec![]
            }
        );
    }
}

#[test]
fn open_folder_remains_the_rename_target_until_its_pane_has_a_selection() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    browser.preview(0, 0);

    let (depth, position, entry) = browser.rename_item().expect("open folder rename target");
    assert_eq!((depth, position), (0, 0));
    assert_eq!(entry.location, Location::local("/fixture/child"));
}

#[test]
fn native_selection_notifies_observers_after_state_is_available() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    browser.navigate(Location::local("/fixture"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    let weak_browser = Rc::downgrade(&browser);
    browser.observe(move |event| {
        let BrowserEvent::SelectionSynced { depth, focused } = event else {
            panic!("native selection must not request focus or reapply view selection: {event:?}");
        };
        let browser = weak_browser.upgrade().expect("browser");
        observed
            .borrow_mut()
            .push((*depth, *focused, browser.selected_positions(*depth)));
    });

    browser.set_selection(0, &[0], Some(0));
    browser.set_selection(0, &[], None);
    browser.set_selection(0, &[1], Some(1));
    browser.set_selection(1, &[0], Some(0));

    assert_eq!(
        *events.borrow(),
        vec![(0, Some(0), vec![0]), (0, None, vec![])],
        "invalid selections must not notify observers"
    );
}

#[test]
fn escape_clears_only_the_active_selection_and_preserves_the_cursor() {
    for multiple in [false, true] {
        let mut source = ScriptedSource::scripted(vec!["a.txt", "b.txt"], Vec::new());
        source.dirs = vec!["child"];
        let browser = Browser::new(Rc::new(source));
        browser.navigate(Location::local("/fixture"));
        browser.activate_focused();
        if multiple {
            browser.select_all(1);
        }
        let parent_selection = browser.selected_positions(0);
        let focused = browser.focused_item();
        let location = browser.active_location();
        assert_eq!(
            browser.selected_positions(1).len(),
            if multiple { 3 } else { 1 }
        );

        browser.escape();

        assert!(browser.selected_positions(1).is_empty());
        assert_eq!(browser.selected_positions(0), parent_selection);
        assert_eq!(browser.focused_item(), focused);
        assert_eq!(browser.active_location(), location);
        assert!(!browser.selection_is_load_cursor());
        browser.move_selection(1);
        assert!(!browser.selected_positions(1).is_empty());
    }
}

#[test]
fn select_all_excludes_hidden_entries_unless_shown() {
    let source = ScriptedSource::scripted(vec!["visible.txt", ".hidden.txt"], Vec::new());
    let browser = Browser::new(Rc::new(source));
    browser.navigate(Location::local("/fixture"));

    browser.select_all(0);
    let selected = browser.selected_positions(0);
    assert_eq!(selected.len(), 1, "{selected:?}");
    let entry = browser.entry_at(0, selected[0]).expect("selected entry");
    assert_eq!(entry.display_name, "visible.txt");

    browser.toggle_hidden();
    browser.select_all(0);
    assert_eq!(browser.selected_positions(0).len(), 2);
}

#[test]
fn repeated_identical_batches_emit_selection_only_once() {
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
    let batch = |names: &[&str]| DirectoryEvent::Batch {
        request_id,
        entries: names.iter().map(|name| batch_entry(name)).collect(),
    };
    emit(batch(&["alpha", "beta"]));
    browser.select(0, 0);
    emit(batch(&["mike"]));
    emit(batch(&["zulu"]));
    browser.flush_coalesced_capped(None);

    let selections: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::SelectionSetChanged {
                positions, focused, ..
            } => Some((positions.clone(), *focused)),
            _ => None,
        })
        .collect();
    assert_eq!(selections, vec![(vec![0], 0)]);
    let inserted = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::EntriesInserted { .. }))
        .count();
    assert_eq!(inserted, 2);
}
