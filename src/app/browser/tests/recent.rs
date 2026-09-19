// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, MetadataValue, SortDirection, SortKey, ViewPreferences};
use crate::services::DirectoryEvent;
use std::{cell::RefCell, rc::Rc};

fn recent_entry(name: &str, kind: EntryKind, recent: i64, modified: i64) -> FileEntry {
    FileEntry {
        kind,
        recent_unix_seconds: MetadataValue::Known(recent),
        modified_unix_seconds: MetadataValue::Known(modified),
        ..batch_entry(name)
    }
}

struct ReloadingRecentSource {
    entries: Rc<RefCell<Vec<FileEntry>>>,
    notify: Rc<RefCell<Option<WatchCallback>>>,
}

impl FileSource for ReloadingRecentSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: self.entries.borrow().clone(),
        });
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
        _location: Location,
        _include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.notify.replace(Some(notify));
        Some(LoadHandle::new(|| {}))
    }
}

fn load_recent(
    entries: Vec<FileEntry>,
) -> (Rc<Browser>, Rc<RefCell<Vec<BrowserEvent>>>, CapturedLoad) {
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::new(Rc::new(BatchReplaySource {
        captured: captured.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(Location::uri("recent:///"));
    let (request_id, emit) = captured.borrow().clone().expect("Recent load");
    emit(DirectoryEvent::Batch {
        request_id,
        entries,
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    browser.flush_coalesced_capped(None);

    (browser, events, captured)
}

#[test]
fn recent_monitor_rescan_reloads_targets_and_preserves_local_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let recent = Location::uri("recent:///");
    let entries = Rc::new(RefCell::new(vec![
        recent_entry("a", EntryKind::File, 20, 1),
        recent_entry("b", EntryKind::File, 10, 2),
    ]));
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(ReloadingRecentSource {
        entries: entries.clone(),
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    browser.navigate(recent.clone());
    browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
    pump_until(|| {
        browser.column_preferences(0).is_some_and(|preferences| {
            preferences.sort_key == SortKey::Name
                && preferences.sort_direction == SortDirection::Ascending
        })
    });
    events.borrow_mut().clear();

    entries.replace(vec![
        recent_entry("b", EntryKind::File, 10, 2),
        recent_entry("c", EntryKind::File, 30, 3),
    ]);
    notify
        .borrow()
        .clone()
        .expect("Recent monitor should be installed")(DirectoryChange::Rescan);

    assert_eq!(browser.location_at(0), Some(recent));
    assert_eq!(column_names(&browser, 0), ["b", "c"]);
    assert_eq!(
        browser
            .entry_at(0, 0)
            .expect("reloaded Recent entry")
            .location,
        Location::local("/fixture/b")
    );
    assert_eq!(
        browser.column_preferences(0).map(|preferences| (
            preferences.sort_key,
            preferences.sort_direction,
            preferences.folders_first,
        )),
        Some((SortKey::Name, SortDirection::Ascending, false))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

#[test]
fn recent_columns_start_with_recency_descending_and_folders_first_disabled() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, _) = load_recent(Vec::new());

    assert_eq!(
        browser.column_preferences(0),
        Some(ViewPreferences {
            sort_key: SortKey::Recency,
            sort_direction: SortDirection::Descending,
            folders_first: false,
            ..ViewPreferences::default()
        })
    );
}

#[test]
fn recent_default_order_uses_recent_use_time_instead_of_modified_time() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, _) = load_recent(vec![
        recent_entry("old-use-new-mtime", EntryKind::File, 10, 300),
        recent_entry("new-use-old-mtime", EntryKind::File, 20, 1),
        recent_entry("old-folder", EntryKind::Directory, 5, 400),
    ]);

    assert_eq!(
        column_names(&browser, 0),
        ["new-use-old-mtime", "old-use-new-mtime", "old-folder"]
    );
}

#[test]
fn recent_recency_sort_supports_ascending_and_descending_directions() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, _) = load_recent(vec![
        recent_entry("new", EntryKind::File, 20, 1),
        recent_entry("old", EntryKind::File, 10, 300),
    ]);

    browser.set_sort(0, SortKey::Recency, SortDirection::Ascending);
    pump_until(|| finish_count(&events) == 1);
    assert_eq!(column_names(&browser, 0), ["old", "new"]);

    browser.set_sort_direction(0, SortDirection::Descending);
    pump_until(|| {
        browser
            .column_preferences(0)
            .is_some_and(|preferences| preferences.sort_direction == SortDirection::Descending)
    });
    assert_eq!(column_names(&browser, 0), ["new", "old"]);
}

#[test]
fn recent_sort_changes_stay_local_and_normal_folder_defaults_are_preserved() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let defaults = ViewPreferences {
        folders_first: true,
        sort_key: SortKey::Name,
        sort_direction: SortDirection::Ascending,
        ..ViewPreferences::default()
    };
    let captured: CapturedLoad = Rc::new(RefCell::new(None));
    let browser = Browser::with_preferences(
        Rc::new(BatchReplaySource {
            captured: captured.clone(),
        }),
        defaults,
    );
    let observed = Rc::new(RefCell::new(Vec::new()));
    let observed_preferences = observed.clone();
    browser.observe_preferences(move |preferences| {
        observed_preferences.borrow_mut().push(preferences);
    });

    browser.navigate(Location::local("/fixture"));
    assert_eq!(browser.column_preferences(0), Some(defaults));

    browser.navigate(Location::uri("recent:///"));
    browser.set_sort(0, SortKey::Size, SortDirection::Ascending);
    pump_until(|| {
        browser.column_preferences(0).is_some_and(|preferences| {
            preferences.sort_key == SortKey::Size
                && preferences.sort_direction == SortDirection::Ascending
        })
    });
    browser.set_sort_direction(0, SortDirection::Descending);
    pump_until(|| {
        browser
            .column_preferences(0)
            .is_some_and(|preferences| preferences.sort_direction == SortDirection::Descending)
    });
    browser.set_folders_first(0, true);
    gtk::glib::MainContext::default().iteration(true);
    assert!(
        !browser
            .column_preferences(0)
            .expect("Recent preferences")
            .folders_first
    );

    assert_eq!(browser.preferences(), defaults);
    assert!(observed.borrow().is_empty());

    browser.navigate(Location::local("/fixture/after-recent"));
    assert_eq!(browser.column_preferences(0), Some(defaults));
}

#[test]
fn recency_is_rejected_for_ordinary_locations() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, _) = load_recent(Vec::new());
    browser.navigate(Location::local("/fixture"));
    let defaults = browser
        .column_preferences(0)
        .expect("ordinary folder preferences");

    browser.set_sort_key(0, SortKey::Recency);
    browser.set_sort(0, SortKey::Recency, SortDirection::Descending);

    assert_eq!(browser.column_preferences(0), Some(defaults));
}

#[test]
fn recent_sort_selection_survives_reload() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, captured) = load_recent(vec![
        recent_entry("new", EntryKind::File, 20, 1),
        recent_entry("old", EntryKind::File, 10, 300),
    ]);
    browser.set_sort(0, SortKey::Recency, SortDirection::Ascending);
    pump_until(|| {
        browser
            .column_preferences(0)
            .is_some_and(|preferences| preferences.sort_direction == SortDirection::Ascending)
    });

    browser.retry_column(0);
    let (request_id, emit) = captured.borrow().clone().expect("reloaded Recent request");
    emit(DirectoryEvent::Batch {
        request_id,
        entries: vec![
            recent_entry("new", EntryKind::File, 20, 1),
            recent_entry("old", EntryKind::File, 10, 300),
        ],
    });
    emit(DirectoryEvent::Finished {
        request_id,
        truncated: false,
        can_trash: None,
        can_delete: None,
    });
    browser.flush_coalesced_capped(None);

    assert_eq!(column_names(&browser, 0), ["old", "new"]);
    assert_eq!(
        browser.column_preferences(0).map(|preferences| (
            preferences.sort_key,
            preferences.sort_direction,
            preferences.folders_first,
        )),
        Some((SortKey::Recency, SortDirection::Ascending, false))
    );
}
