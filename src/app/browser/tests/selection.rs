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
fn reloading_a_column_announces_before_clearing_it() {
    for reveal in [false, true] {
        let browser = Browser::new(Rc::new(RestoredSortingSource));
        browser.navigate(Location::local("/fixture"));
        browser.set_selection(0, &[0, 1], Some(1));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let weak = Rc::downgrade(&browser);
        let events = observed.clone();
        browser.observe(move |event| match event {
            BrowserEvent::ColumnReloading { depth } => {
                let browser = weak.upgrade().expect("browser");
                events.borrow_mut().push(format!(
                    "reloading {depth}: {} selected, cursor {:?}",
                    browser.selected_entries().len(),
                    browser
                        .focused_item()
                        .map(|(depth, position, _)| (depth, position))
                ));
            }
            BrowserEvent::ColumnReloaded { depth } => {
                events.borrow_mut().push(format!("reloaded {depth}"));
            }
            _ => {}
        });

        if reveal {
            browser.reveal_locations(
                Location::local("/fixture"),
                vec![Location::local("/fixture/unlisted")],
            );
        } else {
            browser.reload_active();
        }

        // An explicit target outranks a restored position, so nothing is announced.
        let expected: &[&str] = if reveal {
            &["reloaded 0"]
        } else {
            &["reloading 0: 2 selected, cursor Some((0, 1))", "reloaded 0"]
        };
        assert_eq!(*observed.borrow(), expected, "reveal {reveal}");
    }
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
fn cursor_fill_stays_out_of_the_other_column_and_the_open_path() {
    let mut source = ScriptedSource::scripted(vec!["a.txt", "b.txt"], Vec::new());
    source.dirs = vec!["empty", "child"];
    let browser = Rc::new(Browser::new(Rc::new(source)));
    browser.navigate(Location::local("/fixture"));
    assert!(browser.selection_is_load_cursor());
    assert_eq!(browser.toggle_cursor_fill(), CursorToggle::Added);
    assert!(!browser.selection_is_load_cursor());
    let added = browser.focused_entry().expect("cursor").display_name;
    browser.page_cursor(1, 1, None);
    assert_eq!(
        browser
            .selected_entries()
            .into_iter()
            .map(|entry| entry.display_name)
            .collect::<Vec<_>>(),
        vec![added.clone()]
    );
    assert_ne!(
        browser.focused_entry().expect("moved cursor").display_name,
        added
    );

    browser.select_visible(0);
    let all = browser.selected_positions(0);
    assert!(all.len() > 1);
    let cursor = browser.focused_item().map(|(_, position, _)| position);
    browser.page_cursor(1, 1, None);
    assert_eq!(browser.selected_positions(0), all);
    assert_ne!(
        browser.focused_item().map(|(_, position, _)| position),
        cursor
    );

    browser.invert_visible(0);
    let inverted = browser.selected_positions(0);
    assert_ne!(inverted, all);
    browser.page_cursor(1, 1, None);
    assert_eq!(browser.selected_positions(0), inverted);

    let parent = browser.selected_positions(0);
    browser.show_child(0, Location::local("/fixture/child"));
    assert!(browser.column_snapshot(1).is_some());
    browser.select_visible(1);
    assert_eq!(browser.selected_positions(0), parent);
    browser.invert_visible(1);
    assert_eq!(browser.selected_positions(0), parent);
}

#[test]
fn visual_key_repeats_leave_the_range_and_publish_the_walked_fill() {
    let source = ScriptedSource::scripted(vec!["a.txt", "b.txt", "c.txt"], Vec::new());
    let browser = Rc::new(Browser::new(Rc::new(source)));
    browser.navigate(Location::local("/fixture"));
    let fills = Rc::new(RefCell::new(Vec::new()));
    let observed = fills.clone();
    browser.observe(move |event| {
        if let BrowserEvent::SelectionSetChanged {
            selection: SelectionUpdate::Positions(positions),
            focused,
            ..
        } = event
        {
            observed.borrow_mut().push((positions.clone(), *focused));
        }
    });
    let names = |browser: &Browser| {
        let mut names: Vec<_> = browser
            .selected_entries()
            .into_iter()
            .map(|entry| entry.display_name)
            .collect();
        names.sort();
        names
    };
    let reversed = [2, 1, 0];

    assert!(browser.toggle_visual(VisualKind::Select, Some(&reversed)));
    assert_eq!(browser.visual_kind(), Some(VisualKind::Select));
    assert_eq!(names(&browser), ["a.txt"]);
    browser.page_cursor(-1, 1, Some(&reversed));
    assert_eq!(
        browser.focused_entry().map(|entry| entry.display_name),
        Some("b.txt".to_owned())
    );
    assert_eq!(names(&browser), ["a.txt", "b.txt"]);
    browser.place_cursor(0, 2, Some(&reversed));
    assert_eq!(names(&browser), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(fills.borrow().last(), Some(&(vec![0, 1, 2], 2)));

    assert!(browser.toggle_visual(VisualKind::Select, Some(&reversed)));
    assert_eq!(browser.visual_kind(), None, "repeating v leaves the range");
    browser.place_cursor(0, 0, Some(&reversed));
    assert_eq!(names(&browser), ["a.txt", "b.txt", "c.txt"]);

    assert!(browser.toggle_visual(VisualKind::Unset, Some(&reversed)));
    assert_eq!(names(&browser), ["b.txt", "c.txt"]);
    assert!(browser.toggle_visual(VisualKind::Select, Some(&reversed)));
    assert_eq!(
        browser.visual_kind(),
        Some(VisualKind::Select),
        "the other key restarts at the cursor"
    );
    assert_eq!(names(&browser), ["a.txt", "b.txt", "c.txt"]);
    let published = fills.borrow().len();
    assert!(browser.leave_visual());
    assert_eq!(
        fills.borrow().len(),
        published + 1,
        "leaving republishes the fill"
    );
    assert!(!browser.leave_visual());
    assert_eq!(names(&browser), ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn select_all_excludes_hidden_entries_unless_shown() {
    let source = ScriptedSource::scripted(vec!["visible.txt", ".hidden.txt"], Vec::new());
    let browser = Browser::new(Rc::new(source));
    browser.navigate(Location::local("/fixture"));

    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.select_all(0);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            selection: SelectionUpdate::All,
            ..
        }
    )));
    let selected = browser.selected_positions(0);
    assert_eq!(selected.len(), 1, "{selected:?}");
    let entry = browser.entry_at(0, selected[0]).expect("selected entry");
    assert_eq!(entry.display_name, "visible.txt");

    browser.toggle_hidden();
    browser.select_all(0);
    assert_eq!(browser.selected_positions(0).len(), 2);
}

#[test]
fn deleting_an_entry_respects_neighbor_visibility_and_cursor_fill() {
    for (names, removed, visible_neighbor) in [
        (vec!["alpha.txt", ".hidden.txt"], "alpha.txt", None),
        (
            vec!["alpha.txt", ".hidden.txt", "charlie.txt"],
            "alpha.txt",
            Some("charlie.txt"),
        ),
        (
            vec!["alpha.txt", ".hidden.txt", "charlie.txt"],
            "charlie.txt",
            Some("alpha.txt"),
        ),
    ] {
        for show_hidden in [false, true] {
            for batched in [false, true] {
                for preserve_fill in [false, true] {
                    let source = ScriptedSource::scripted(Vec::new(), Vec::new());
                    let browser = Rc::new(Browser::new(Rc::new(source)));
                    browser.apply_default_preferences(ViewPreferences {
                        sort_key: SortKey::Size,
                        ..ViewPreferences::default()
                    });
                    let parent = Location::local("/fixture");
                    browser.navigate(parent.clone());
                    if show_hidden {
                        browser.toggle_hidden();
                    }
                    browser.set_preserve_fill_on_removal(preserve_fill);
                    for (position, name) in names.iter().enumerate() {
                        let mut entry = batch_entry(name);
                        entry.is_hidden = name.starts_with('.');
                        entry.size = MetadataValue::Known(position as u64);
                        browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(entry));
                    }
                    let order = column_names(&browser, 0);
                    assert_eq!(order, names);
                    let focused = order
                        .iter()
                        .position(|name| name == removed)
                        .expect("deleted entry must be listed");
                    browser.select(0, focused);
                    let change =
                        DirectoryChange::Remove(Location::local(format!("/fixture/{removed}")));
                    if batched {
                        browser.flush_deferred_file_operation_changes(
                            std::collections::HashMap::from([(0, vec![(parent.clone(), change)])]),
                            false,
                        );
                    } else {
                        browser.handle_directory_change(0, &parent, change);
                    }

                    let expected = if show_hidden {
                        Some(".hidden.txt")
                    } else {
                        visible_neighbor
                    };
                    let context = format!(
                        "removing {removed} from {order:?}: hidden={show_hidden}, batch={batched}, preserve={preserve_fill}"
                    );
                    assert_eq!(
                        browser
                            .focused_entry()
                            .map(|entry| entry.display_name)
                            .as_deref(),
                        expected,
                        "{context}"
                    );
                    let selected: Vec<_> = browser
                        .selected_entries()
                        .into_iter()
                        .map(|entry| entry.display_name)
                        .collect();
                    let expected_selected: Vec<_> =
                        expected.filter(|_| !preserve_fill).into_iter().collect();
                    assert_eq!(selected, expected_selected, "{context}");
                }
            }
        }
    }
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
                selection: SelectionUpdate::Positions(positions),
                focused,
                ..
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
