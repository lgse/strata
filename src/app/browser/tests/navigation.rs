// SPDX-License-Identifier: MIT

use super::*;

struct BoundarySource(RecordingFileSource);

impl FileSource for BoundarySource {
    fn allows_navigation(&self, location: &Location) -> bool {
        location
            .native_path()
            .is_some_and(|path| path.starts_with("/fixture/device"))
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        self.0.validate_location(location)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.0.enumerate(request, emit)
    }
}

#[test]
fn navigation_boundary_blocks_trusted_routes_without_loading_outside_locations() {
    let requests = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(BoundarySource(RecordingFileSource {
        request_count: requests.clone(),
    })));
    let root = Location::local("/fixture/device");
    let child = Location::local("/fixture/device/child");
    let outside = Location::local("/fixture/outside");
    browser.navigate(root.clone());
    assert!(!browser.can_go_parent());
    browser.parent();
    browser.navigate(outside.clone());
    browser.descend(0, outside.clone());
    browser.show_child(0, outside);
    assert_eq!(browser.active_location(), Some(root.clone()));
    assert_eq!(
        requests.get(),
        1,
        "rejected navigation never enumerates an outside location"
    );
    assert!(
        !browser.can_go_back(),
        "rejected routes do not enter navigation history"
    );
    browser.navigate(child.clone());
    assert!(browser.can_go_parent());
    browser.parent();
    assert_eq!(browser.active_location(), Some(root.clone()));
    browser.back();
    assert_eq!(browser.active_location(), Some(child));
    browser.forward();
    assert_eq!(browser.active_location(), Some(root));
}

#[test]
fn column_snapshots_preserve_load_errors() {
    let browser = Browser::new(Rc::new(RetryFileSource {
        attempts: Rc::new(Cell::new(0)),
    }));

    browser.navigate(Location::local("/fixture"));

    let snapshot = browser.column_snapshot(0).expect("column should exist");
    assert_eq!(snapshot.error.as_deref(), Some("temporarily unavailable"));
    assert!(!snapshot.loading);
}

#[test]
fn retrying_a_failed_column_preserves_navigation_history() {
    let attempts = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RetryFileSource {
        attempts: attempts.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.retry_column(0);

    assert_eq!(attempts.get(), 2);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesReplaced { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::Reset))
    );
}

#[test]
fn navigating_away_cancels_the_previous_directory_request() {
    let cancellations = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(TrackingFileSource {
        cancellations: cancellations.clone(),
    }));

    browser.navigate(Location::local("/first"));
    browser.navigate(Location::local("/second"));

    assert_eq!(cancellations.get(), 1);
}

#[test]
fn navigating_to_the_active_location_is_a_noop() {
    let cancellations = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(TrackingFileSource {
        cancellations: cancellations.clone(),
    }));
    let resets = Rc::new(Cell::new(0));
    let observed_resets = resets.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_resets.set(observed_resets.get() + 1);
        }
    });

    browser.navigate(Location::uri("trash:///"));
    browser.navigate(Location::uri("trash:///"));

    assert_eq!(cancellations.get(), 0);
    assert_eq!(resets.get(), 1);
}

#[test]
fn recent_is_consumed_as_a_parentless_browser_location() {
    let browser = Browser::new(Rc::new(ScriptedSource::scripted(
        vec!["target.txt"],
        vec![],
    )));
    let recent = Location::uri("recent:///");

    browser.navigate(Location::local("/fixture"));
    browser.navigate(recent.clone());

    assert_eq!(browser.active_location(), Some(recent.clone()));
    assert!(!browser.can_go_parent());
    assert_eq!(
        browser
            .entry_at(0, 0)
            .expect("Recent should publish a normal entry")
            .location,
        Location::local("/fixture/target.txt")
    );

    browser.back();
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    browser.forward();
    assert_eq!(browser.active_location(), Some(recent));
}

#[test]
fn empty_recent_load_finishes_as_an_empty_column() {
    let browser = Browser::new(Rc::new(ScriptedSource::scripted(vec![], vec![])));

    browser.navigate(Location::uri("recent:///"));

    let snapshot = browser.column_snapshot(0).expect("Recent column");
    assert_eq!(snapshot.count, 0);
    assert!(!snapshot.loading);
    assert_eq!(snapshot.error, None);
}

#[test]
fn file_source_can_be_replaced_without_constructing_the_ui() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::local("/fixture"));

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesReplaced { count: 1, .. }))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
}

#[test]
fn peeking_streams_results_without_committing_navigation_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    browser.begin_peek(0, Location::local("/fixture/child"));

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekStarted { .. }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::PeekEntriesAdded { entries } if entries.len() == 1
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekFinished))
    );

    browser.back();
    let resets = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::Reset))
        .count();
    assert_eq!(resets, 1, "a peek must not create a history entry");
}

#[test]
fn an_already_open_child_is_not_peeked() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    let child = Location::local("/fixture/child");
    browser.navigate(Location::local("/fixture"));
    browser.descend(0, child.clone());
    events.borrow_mut().clear();

    browser.begin_peek(0, child);

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekStarted { .. }))
    );
}

#[test]
fn committing_a_peek_descends_and_creates_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.begin_peek(0, Location::local("/fixture/child"));

    browser.commit_peek();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnAdded { depth: 1, location }
            if location == &Location::local("/fixture/child")
    )));
    browser.back();
    let resets = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::Reset))
        .count();
    assert_eq!(resets, 2, "committing a peek must create a history entry");
}

#[test]
fn single_click_action_descends_into_directories() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn activating_an_open_list_item_closes_its_child_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.preview(0, 0);
    assert_eq!(browser.active_depth(), Some(1));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { len: 1 }))
    );
    assert_eq!(browser.active_depth(), Some(0));
}

#[test]
fn requesting_first_selection_during_navigate_selects_the_first_entry() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let loader = browser.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::ColumnAdded { depth: 0, .. }) {
            loader.select_first_on_load(0);
        }
    });

    browser.navigate(Location::local("/fixture"));

    assert_eq!(browser.selected_positions(0), [0]);
}

#[test]
fn list_activation_replaces_the_directory_instead_of_adding_a_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.activate_in_place(0, 0);

    assert_eq!(
        browser.active_location(),
        Some(Location::local("/fixture/child"))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn directory_navigation_does_not_open_or_preview_files() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.enter_focused_directory();
    let focused = browser.focused_item();
    assert!(focused.is_some());
    events.borrow_mut().clear();

    for _ in 0..3 {
        browser.enter_focused_directory();
    }
    assert!(events.borrow().is_empty());
    assert_eq!(browser.focused_item(), focused);
    assert!(browser.location_at(1).is_none());

    browser.activate_focused();
    assert!(events.borrow().iter().any(|event| matches!(event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/example.conf")
    )));
}

#[test]
fn directory_navigation_moves_right_from_a_file_into_an_open_column() {
    let browser = Browser::new(Rc::new(OpenChildBesideFileSource));
    browser.navigate(Location::local("/fixture"));
    browser.select(0, 0);
    browser.enter_focused_directory();
    assert_eq!(browser.active_depth(), Some(1));

    browser.focus_parent();
    browser.select(0, 1);
    browser.enter_focused_directory();
    assert_eq!(browser.active_depth(), Some(1));

    browser.focus_parent();
    browser.close_column(1);
    browser.enter_focused_directory();
    assert_eq!(browser.active_depth(), Some(0));
    assert!(browser.location_at(1).is_none());
}

#[test]
fn directory_navigation_enters_and_reuses_folder_columns() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    browser.enter_focused_directory();
    assert!(browser.location_at(2).is_none());
    assert_eq!(browser.active_depth(), Some(1));
    assert_eq!(
        browser.active_location(),
        Some(Location::local("/fixture/child"))
    );

    browser.focus_parent();
    browser.enter_focused_directory();
    assert!(browser.location_at(2).is_none());
    assert_eq!(browser.active_depth(), Some(1));
}

#[test]
fn keyboard_selection_and_activation_descend_without_the_ui() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    browser.move_selection(1);
    browser.activate_focused();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::FocusChanged {
            depth: 0,
            position: Some(0)
        }
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );

    browser.focus_parent();
    events.borrow_mut().clear();
    browser.activate_focused();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::FocusChanged {
            depth: 1,
            position: Some(0)
        }
    )));
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnsTruncated { .. } | BrowserEvent::ColumnAdded { .. }
    )));
}

#[test]
fn escape_closes_a_peek_before_clearing_selection_and_closing_the_deepest_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    browser.activate_focused();
    browser.begin_peek(1, Location::local("/fixture/child/child"));
    events.borrow_mut().clear();

    browser.escape();
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekClosed))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { .. }))
    );

    events.borrow_mut().clear();
    let location = browser.active_location();
    browser.escape();
    assert!(browser.selected_positions(1).is_empty());
    assert_eq!(browser.active_location(), location);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            depth: 1,
            selection: SelectionUpdate::Positions(positions),
            ..
        } if positions.is_empty()
    )));

    events.borrow_mut().clear();
    browser.escape();
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { len: 1 }))
    );
}

#[test]
fn peek_filters_hidden_entries_before_item_limit() {
    let browser = Browser::new(Rc::new(MixedPeekFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.begin_peek(0, Location::local("/fixture/peek_target"));

    let peek_batch = events.borrow().iter().find_map(|event| match event {
        BrowserEvent::PeekEntriesAdded { entries } => Some(entries.clone()),
        _ => None,
    });
    let entries = peek_batch.expect("peek batch emitted");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].display_name, "normal.txt");

    events.borrow_mut().clear();
    browser.toggle_hidden();
    browser.begin_peek(0, Location::local("/fixture/peek_target"));

    let peek_batch = events.borrow().iter().find_map(|event| match event {
        BrowserEvent::PeekEntriesAdded { entries } => Some(entries.clone()),
        _ => None,
    });
    let entries = peek_batch.expect("peek batch emitted");
    assert_eq!(entries.len(), 2);
}

#[test]
fn preview_and_open_are_distinct_file_actions() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::PreviewRequested {
            entry,
            automatic: false
        } if entry.location == Location::local("/fixture/example.conf")
    )));
    events.borrow_mut().clear();

    browser.activate(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/example.conf")
    )));
}

#[test]
fn previewing_a_file_in_a_parent_column_closes_deeper_columns_before_requesting() {
    let browser = Browser::new(Rc::new(OpenChildBesideFileSource));
    browser.navigate(Location::local("/fixture"));
    browser.select(0, 0);
    browser.enter_focused_directory();
    assert_eq!(browser.active_depth(), Some(1));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.preview(0, 1);

    assert!(browser.location_at(1).is_none());
    assert_eq!(browser.active_depth(), Some(0));
    let events = events.borrow();
    let focused_file = events.iter().rposition(|event| {
        matches!(
            event,
            BrowserEvent::FocusChanged {
                depth: 0,
                position: Some(1)
            }
        )
    });
    let requested = events.iter().position(|event| {
        matches!(
            event,
            BrowserEvent::PreviewRequested {
                entry,
                automatic: false
            } if entry.location == Location::local("/fixture/example.conf")
        )
    });
    assert!(
        focused_file
            .zip(requested)
            .is_some_and(|(focus, request)| focus < request),
        "the closed column must report the file's focus before the preview request"
    );
}

#[derive(Default)]
struct FolderTree {
    listings: RefCell<HashMap<Location, Vec<FileEntry>>>,
}

impl FolderTree {
    fn with(folders: &[(&str, &[&str])]) -> Self {
        let tree = Self::default();
        for (directory, children) in folders {
            tree.list(directory, children);
        }
        tree
    }

    fn list(&self, directory: &str, children: &[&str]) {
        let entries = children
            .iter()
            .map(|name| FileEntry {
                kind: EntryKind::Directory,
                ..super::location_input::listing_file(
                    tree_location(&format!("{directory}/{name}")),
                    name,
                    name.starts_with('.'),
                )
            })
            .collect();
        self.listings
            .borrow_mut()
            .insert(tree_location(directory), entries);
    }
}

/// Paths with a scheme are remote, which load in batches instead of a sorted snapshot.
fn tree_location(path: &str) -> Location {
    if path.contains("://") {
        Location::uri(path)
    } else {
        Location::local(path)
    }
}

impl FileSource for FolderTree {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let entries = self
            .listings
            .borrow()
            .get(&request.location)
            .cloned()
            .unwrap_or_default();
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries,
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn folder_browser(
    tree: FolderTree,
) -> (
    Rc<Browser>,
    Rc<RefCell<Vec<BrowserEvent>>>,
    Rc<FolderTree>,
) {
    let tree = Rc::new(tree);
    let browser = Browser::new(tree.clone());
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    (browser, events, tree)
}

fn focused_location(browser: &Browser) -> Option<Location> {
    browser.focused_entry().map(|entry| entry.location)
}

#[derive(Clone, Copy, Debug)]
enum Return {
    Back,
    Parent,
    Forward,
    Ancestor,
}

#[test]
fn returning_to_an_ancestor_selects_the_folder_you_came_from() {
    let folders: &[(&str, &[&str])] = &[
        ("/fx", &["aa", "docs"]),
        ("/fx/docs", &["a1", "b2", "c3", "sub"]),
        ("/fx/docs/b2", &["inner"]),
        ("/fx/docs/sub", &["leaf"]),
    ];
    let single = |browser: &Rc<Browser>| {
        browser.navigate(Location::local("/fx/docs"));
        browser.navigate(Location::local("/fx/docs/b2"));
        (0usize, Location::local("/fx/docs/b2"))
    };
    let nested = |browser: &Rc<Browser>| {
        browser.navigate(Location::local("/fx"));
        browser.descend(0, Location::local("/fx/docs"));
        browser.descend(1, Location::local("/fx/docs/sub"));
        (1usize, Location::local("/fx/docs/sub"))
    };
    let mut failures = Vec::new();
    for (shape, enter) in [
        ("single column", &single as &dyn Fn(&Rc<Browser>) -> (usize, Location)),
        ("nested", &nested),
    ] {
        for route in [
            Return::Back,
            Return::Parent,
            Return::Forward,
            Return::Ancestor,
        ] {
            let (browser, events, _) = folder_browser(FolderTree::with(folders));
            let (entered_depth, child) = enter(&browser);
            let destination = child.parent().expect("parent");
            if matches!(route, Return::Forward) {
                browser.parent();
                browser.back();
                assert_eq!(browser.active_location(), Some(child.clone()));
            }
            events.borrow_mut().clear();
            match route {
                Return::Back => browser.back(),
                Return::Parent => browser.parent(),
                Return::Forward => browser.forward(),
                Return::Ancestor => browser.navigate_to_ancestor(destination.clone()),
            }
            // A breadcrumb opens the ancestor as the only column.
            let depth = match route {
                Return::Ancestor => 0,
                _ => entered_depth,
            };
            pump_until(|| {
                browser
                    .column_snapshot(depth)
                    .is_some_and(|snapshot| !snapshot.loading)
            });

            let case = format!("{shape}, {route:?}");
            assert_eq!(browser.active_location(), Some(destination), "{case}");
            let focused = focused_location(&browser);
            let selected = super::location_input::selected_locations(&browser);
            if focused.as_ref() != Some(&child) || selected != [child.clone()] {
                failures.push(format!(
                    "{case}: cursor {focused:?} and selection {selected:?}, not {child:?}"
                ));
            }
            if events.borrow().iter().any(|event| {
                matches!(
                    event,
                    BrowserEvent::ColumnAdded { depth: added, .. } if *added == depth + 1
                )
            }) {
                failures.push(format!("{case}: the child column reopened"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn returning_to_a_non_ancestor_keeps_the_first_entry() {
    let (browser, ..) = folder_browser(FolderTree::with(&[
        ("/a", &["a1", "a2"]),
        ("/b", &["b1", "b2"]),
    ]));
    browser.navigate(Location::local("/a"));
    browser.navigate(Location::local("/b"));

    browser.back();

    assert_eq!(browser.active_location(), Some(Location::local("/a")));
    assert_eq!(focused_location(&browser), Some(Location::local("/a/a1")));
    assert_eq!(super::location_input::selected_locations(&browser), vec![Location::local("/a/a1")]);
}

#[test]
fn a_missing_or_hidden_came_from_child_falls_back_quietly() {
    for (root, child, docs_after_leaving) in [
        ("", "b2", &["a1", "c3"][..]),
        ("", ".b2", &["a1", ".b2", "c3"][..]),
        ("smb://host", "b2", &["a1", "c3"][..]),
    ] {
        let docs = format!("{root}/docs");
        let (browser, events, tree) = folder_browser(FolderTree::with(&[
            (&docs, &["a1", "b2", ".b2", "c3"]),
            (&format!("{docs}/b2"), &["inner"]),
            (&format!("{docs}/.b2"), &["inner"]),
        ]));
        browser.navigate(tree_location(&docs));
        pump_until(|| browser.column_snapshot(0).is_some_and(|snapshot| !snapshot.loading));
        browser.navigate(tree_location(&format!("{docs}/{child}")));
        pump_until(|| browser.column_snapshot(0).is_some_and(|snapshot| !snapshot.loading));
        tree.list(&docs, docs_after_leaving);
        events.borrow_mut().clear();

        browser.back();
        pump_until(|| browser.column_snapshot(0).is_some_and(|snapshot| !snapshot.loading));
        let case = format!("{docs}, {child}");
        let first = tree_location(&format!("{docs}/a1"));
        assert_eq!(browser.active_location(), Some(tree_location(&docs)), "{case}");
        assert_eq!(focused_location(&browser), Some(first.clone()), "{case}");
        assert_eq!(super::location_input::selected_locations(&browser), vec![first], "{case}");
        assert!(!browser.preferences().show_hidden, "{case}: hidden files stay hidden");
        let cursor = browser.focused_item().map(|(_, position, _)| position);
        let events = events.borrow();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, BrowserEvent::LocationRevealFailed { .. })),
            "{case}: no reveal failure is reported"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                BrowserEvent::SelectionSetChanged { depth: 0, focused, .. }
                    if Some(*focused) == cursor
            )),
            "{case}: the fallback selection is published"
        );
    }
}

#[test]
fn a_remote_reload_announces_the_neighbour_cursor_of_an_inactive_column() {
    let (browser, events, tree) = folder_browser(FolderTree::with(&[
        ("smb://host/docs", &["a1", "b2", "c3"]),
        ("smb://host/docs/b2", &["x", "y", "z"]),
    ]));
    let loaded = |depth: usize| {
        browser
            .column_snapshot(depth)
            .is_some_and(|snapshot| !snapshot.loading)
    };
    browser.navigate(tree_location("smb://host/docs"));
    pump_until(|| loaded(0));
    browser.show_child(0, tree_location("smb://host/docs/b2"));
    pump_until(|| loaded(1));
    browser.set_selection(1, &[], Some(1));
    browser.set_active_column(0);
    tree.list("smb://host/docs/b2", &["x", "z"]);
    events.borrow_mut().clear();

    browser.refresh_all();
    pump_until(|| loaded(0) && loaded(1));

    assert!(
        events.borrow().iter().any(|event| matches!(
            event,
            BrowserEvent::SelectionSetChanged { depth: 1, focused: 1, .. }
        )),
        "the child column's cursor moves to z and is published"
    );
}
