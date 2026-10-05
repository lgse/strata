// SPDX-License-Identifier: MIT

use super::*;

struct RecentMonitorSource;

impl FileSource for RecentMonitorSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        crate::adapters::LocalFileSource.watch(location, include_hidden, notify)
    }
}

#[test]
fn recent_navigation_keeps_the_normal_monitor_lifecycle() {
    let browser = Browser::new(Rc::new(RecentMonitorSource));

    browser.navigate(Location::uri("recent:///"));

    assert_eq!(browser.monitors.borrow().len(), 1);
    assert_eq!(browser.location_at(0), Some(Location::uri("recent:///")));
    assert_eq!(browser.column_snapshot(0).expect("Recent column").count, 0);
}

#[test]
fn filesystem_notifications_update_the_affected_column_incrementally() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Upsert(FileEntry {
        location: Location::local("/fixture/added"),
        native_name: OsString::from("added"),
        thumbnail_path: None,
        display_name: "added".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Known(1),
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    }));

    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::FocusChanged { .. }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { depth: 0, splices, .. }
            if splices.len() == 1 && splices[0].removed == 0 && splices[0].entries.len() == 1
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn background_directory_removal_does_not_request_focus() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let parent = Location::local("/fixture");
    browser.navigate(parent.clone());
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("moved")));
    browser.preview(0, 0);
    assert_eq!(browser.active_depth(), Some(1));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

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
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::FocusChanged { .. }))
    );
}

#[test]
fn active_directory_background_change_does_not_request_focus() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let parent = Location::local("/fixture");
    browser.navigate(parent.clone());
    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("alpha")));
    assert_eq!(browser.active_depth(), Some(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.handle_directory_change(0, &parent, DirectoryChange::Upsert(batch_entry("beta")));

    assert_eq!(browser.active_depth(), Some(0));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesSpliced { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::FocusChanged { .. }))
    );
}

#[test]
fn ambiguous_filesystem_notifications_fall_back_to_reload() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Rescan);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}
