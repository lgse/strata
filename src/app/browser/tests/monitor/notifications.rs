// SPDX-License-Identifier: MIT

use super::*;

/// A column on `/fixture` whose directory watcher the test drives by hand.
struct Watched {
    browser: Rc<Browser>,
    events: Events,
    notify: Rc<RefCell<Option<WatchCallback>>>,
}

impl Watched {
    fn open(location: Location) -> Self {
        let notify = Rc::new(RefCell::new(None::<WatchCallback>));
        let browser = Browser::new(Rc::new(WatchingFileSource {
            notify: notify.clone(),
        }));
        let events = recorded_events(&browser);
        browser.navigate(location);
        Self {
            browser,
            events,
            notify,
        }
    }

    fn report(&self, change: DirectoryChange) {
        self.events.borrow_mut().clear();
        let callback = self
            .notify
            .borrow()
            .clone()
            .expect("the directory watcher should be installed");
        callback(change);
    }
}

fn added_file() -> FileEntry {
    FileEntry {
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Known(1),
        created_unix_seconds: MetadataValue::Unknown,
        ..fixture_entry("/fixture/added")
    }
}

fn single_insertion(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::EntriesSpliced { depth: 0, splices, .. }
            if splices.len() == 1 && splices[0].removed == 0 && splices[0].entries.len() == 1
    )
}

#[test]
fn filesystem_notifications_update_the_affected_column_incrementally() {
    let watched = Watched::open(Location::local("/fixture"));
    watched.browser.move_selection(1);

    watched.report(DirectoryChange::Upsert(added_file()));

    let events = &watched.events;
    assert!(!any_event(events, |event| matches!(
        event,
        BrowserEvent::FocusChanged { .. }
    )));
    assert!(any_event(events, single_insertion));
    assert!(!any_event(events, |event| matches!(
        event,
        BrowserEvent::SelectionSetChanged { .. } | BrowserEvent::ColumnReloaded { .. }
    )));
}

fn browser_listing(name: &str) -> (Rc<Browser>, Location) {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let parent = Location::local("/fixture");
    browser.navigate(parent.clone());
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry(name)));
    (browser, parent)
}

fn assert_spliced_without_selection_or_focus(events: &RefCell<Vec<BrowserEvent>>) {
    assert!(any_event(events, |event| matches!(
        event,
        BrowserEvent::EntriesSpliced { depth: 0, .. }
    )));
    assert!(!any_event(events, |event| matches!(
        event,
        BrowserEvent::SelectionSetChanged { .. }
    )));
    assert!(!any_event(events, |event| matches!(
        event,
        BrowserEvent::FocusChanged { .. }
    )));
}

#[test]
fn background_directory_removal_does_not_request_focus() {
    let (browser, parent) = browser_listing("moved");
    browser.preview(0, 0);
    assert_eq!(browser.active_depth(), Some(1));
    let events = recorded_events(&browser);

    browser.handle_directory_change(
        0,
        &parent,
        DirectoryChange::Remove(Location::local("/fixture/moved")),
    );

    assert_eq!(browser.active_depth(), Some(1));
    assert_eq!(
        browser
            .column_snapshot(0)
            .expect("parent column")
            .selected_positions,
        [0]
    );
    assert_spliced_without_selection_or_focus(&events);
}

#[test]
fn active_directory_background_change_does_not_request_focus() {
    let (browser, parent) = browser_listing("alpha");
    assert_eq!(browser.active_depth(), Some(0));
    let events = recorded_events(&browser);

    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("beta")));

    assert_eq!(browser.active_depth(), Some(0));
    assert_spliced_without_selection_or_focus(&events);
}

#[test]
fn ambiguous_filesystem_notifications_fall_back_to_reload() {
    let watched = Watched::open(Location::local("/fixture"));

    watched.report(DirectoryChange::Rescan);

    assert!(any_event(&watched.events, |event| matches!(
        event,
        BrowserEvent::ColumnReloaded { depth: 0 }
    )));
}
