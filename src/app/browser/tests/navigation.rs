// SPDX-License-Identifier: MIT

use super::*;

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
        BrowserEvent::SelectionSetChanged { depth: 1, positions, .. } if positions.is_empty()
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
        BrowserEvent::PreviewRequested { entry }
            if entry.location == Location::local("/fixture/example.conf")
    )));
    events.borrow_mut().clear();

    browser.activate(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/example.conf")
    )));
}
