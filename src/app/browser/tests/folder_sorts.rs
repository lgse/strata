// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use super::*;
use crate::{app::navigation::FolderSortResolver, model::FolderSort, services::PasteItem};

type SavedSorts = Rc<RefCell<HashMap<Location, FolderSort>>>;
type ReportedSorts = Rc<RefCell<Vec<(Location, SortKey, SortDirection)>>>;

/// Unlisted locations behave like remembered folders without a saved sort.
fn resolver(saved: &SavedSorts) -> FolderSortResolver {
    let saved = saved.clone();
    Rc::new(move |location: &Location| {
        saved
            .borrow()
            .get(location)
            .copied()
            .unwrap_or(FolderSort::Default)
    })
}

fn remembering_browser(
    source: Rc<dyn FileSource>,
    saved: &SavedSorts,
) -> (Rc<Browser>, ReportedSorts) {
    let browser = Browser::with_preferences(source, ViewPreferences::default());
    browser.set_folder_sorts(Some(resolver(saved)));
    let reported = Rc::new(RefCell::new(Vec::new()));
    let observed = reported.clone();
    browser.observe_folder_sorts(move |location, sort_key, sort_direction| {
        observed
            .borrow_mut()
            .push((location.clone(), sort_key, sort_direction));
    });
    (browser, reported)
}

/// Leaves no re-sync queued on the shared default main context, and fails
/// instead of blocking when nothing is left to dispatch.
fn pump_until_settled(browser: &Browser, condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let context = gtk::glib::MainContext::default();
    while !(condition() && browser.sort_resync_settled()) {
        assert!(std::time::Instant::now() < deadline, "timed out");
        if !context.iteration(false) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

fn sorting(browser: &Browser, depth: usize) -> Option<(SortKey, SortDirection)> {
    browser
        .column_preferences(depth)
        .map(|preferences| (preferences.sort_key, preferences.sort_direction))
}

#[test]
fn remembered_folders_open_in_their_saved_sort_and_sorting_reports_only_that_folder() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let saved: SavedSorts = Rc::new(RefCell::new(HashMap::from([(
        Location::local("/fixture"),
        FolderSort::Saved(SortKey::Size, SortDirection::Descending),
    )])));
    let (browser, reported) = remembering_browser(Rc::new(RestoredSortingSource), &saved);
    let defaults_published = Rc::new(Cell::new(0));
    let published = defaults_published.clone();
    browser.observe_preferences(move |_| published.set(published.get() + 1));

    browser.navigate(Location::local("/fixture"));
    browser.descend(0, Location::local("/fixture/child"));

    assert_eq!(
        sorting(&browser, 0),
        Some((SortKey::Size, SortDirection::Descending))
    );
    assert_eq!(column_names(&browser, 0), ["large", "small"]);
    assert_eq!(
        sorting(&browser, 1),
        Some((SortKey::Name, SortDirection::Ascending))
    );

    browser.set_sort(1, SortKey::Type, SortDirection::Descending);
    pump_until_settled(&browser, || {
        sorting(&browser, 1) == Some((SortKey::Type, SortDirection::Descending))
    });

    assert_eq!(
        reported.borrow().as_slice(),
        [(
            Location::local("/fixture/child"),
            SortKey::Type,
            SortDirection::Descending
        )]
    );
    assert_eq!(browser.preferences(), ViewPreferences::default());
    assert_eq!(defaults_published.get(), 0);
    browser.descend(1, Location::local("/fixture/child/next"));
    assert_eq!(
        sorting(&browser, 2),
        Some((SortKey::Name, SortDirection::Ascending)),
        "an unsaved folder opens in the default sort, not the last one chosen"
    );
}

#[test]
fn saved_and_default_changes_resync_open_columns_while_unremembered_columns_keep_their_sort() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let share = Location::uri("smb://host/share");
    let saved: SavedSorts = Rc::new(RefCell::new(HashMap::from([(
        share.clone(),
        FolderSort::Unremembered,
    )])));
    let (folder, reported) = remembering_browser(Rc::new(FakeFileSource), &saved);
    let (remote, _) = remembering_browser(Rc::new(FakeFileSource), &saved);
    folder.navigate(Location::local("/fixture"));
    remote.navigate(share.clone());
    remote.set_sort(0, SortKey::Type, SortDirection::Ascending);
    pump_until_settled(&remote, || {
        sorting(&remote, 0) == Some((SortKey::Type, SortDirection::Ascending))
    });

    saved.borrow_mut().insert(
        Location::local("/fixture"),
        FolderSort::Saved(SortKey::Type, SortDirection::Descending),
    );
    folder.resync_column_sorts();
    pump_until_settled(&folder, || {
        sorting(&folder, 0) == Some((SortKey::Type, SortDirection::Descending))
    });
    assert_eq!(
        sorting(&folder, 0),
        Some((SortKey::Type, SortDirection::Descending))
    );

    let defaults = ViewPreferences {
        sort_direction: SortDirection::Descending,
        ..ViewPreferences::default()
    };
    saved.borrow_mut().remove(&Location::local("/fixture"));
    for browser in [&folder, &remote] {
        browser.apply_default_preferences(defaults);
    }
    pump_until_settled(&folder, || {
        sorting(&folder, 0) == Some((SortKey::Name, SortDirection::Descending))
    });
    assert!(remote.sort_resync_settled());

    assert_eq!(
        sorting(&folder, 0),
        Some((SortKey::Name, SortDirection::Descending))
    );
    assert_eq!(
        sorting(&remote, 0),
        Some((SortKey::Type, SortDirection::Ascending))
    );
    assert!(
        reported.borrow().is_empty(),
        "following a saved or default sort is not a new choice"
    );
}

#[test]
fn a_resync_sort_abandoned_for_missing_metadata_keeps_the_order_without_retrying() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _) = remembering_browser(Rc::new(FakeFileSource), &Rc::default());
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    browser.apply_default_preferences(ViewPreferences {
        sort_key: SortKey::Size,
        ..ViewPreferences::default()
    });
    pump_until_settled(&browser, || true);

    assert_eq!(
        sorting(&browser, 0),
        Some((SortKey::Name, SortDirection::Ascending))
    );
    assert_eq!(start_count(&events), 1);
    assert_eq!(finish_count(&events), 1);
}

#[test]
fn folders_first_is_application_wide_and_resorts_every_open_column() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    for remembering in [true, false] {
        let browser = Browser::new(Rc::new(FakeFileSource));
        if remembering {
            browser.set_folder_sorts(Some(resolver(&Rc::default())));
        }
        let published = Rc::new(RefCell::new(Vec::new()));
        let observed = published.clone();
        browser.observe_preferences(move |preferences| observed.borrow_mut().push(preferences));
        browser.navigate(Location::local("/fixture"));
        browser.descend(0, Location::local("/fixture/child"));

        browser.set_folders_first(1, false);
        pump_until_settled(&browser, || {
            (0..2).all(|depth| {
                browser
                    .column_preferences(depth)
                    .is_some_and(|preferences| !preferences.folders_first)
            })
        });

        for depth in 0..2 {
            assert!(
                !browser
                    .column_preferences(depth)
                    .expect("column")
                    .folders_first,
                "remembering {remembering}, depth {depth}"
            );
        }
        assert!(!browser.preferences().folders_first);
        assert_eq!(published.borrow().len(), 1, "remembering {remembering}");
        browser.descend(1, Location::local("/fixture/child/next"));
        assert!(
            !browser
                .column_preferences(2)
                .expect("new column")
                .folders_first
        );
    }
}

#[test]
fn without_folder_sorts_an_explicit_sort_still_becomes_the_default() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let browser = Browser::new(Rc::new(FakeFileSource));
    let published = Rc::new(RefCell::new(Vec::new()));
    let observed = published.clone();
    browser.observe_preferences(move |preferences| observed.borrow_mut().push(preferences));
    browser.navigate(Location::local("/fixture"));

    browser.set_sort(0, SortKey::Type, SortDirection::Descending);
    pump_until_settled(&browser, || {
        sorting(&browser, 0) == Some((SortKey::Type, SortDirection::Descending))
    });

    let sorted = ViewPreferences {
        sort_key: SortKey::Type,
        sort_direction: SortDirection::Descending,
        ..ViewPreferences::default()
    };
    assert_eq!(browser.preferences(), sorted);
    assert_eq!(published.borrow().as_slice(), [sorted]);
    browser.descend(0, Location::local("/fixture/child"));
    assert_eq!(browser.column_preferences(1), Some(sorted));
}

#[test]
fn renames_moves_and_deletions_report_the_locations_they_changed() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| {
        if matches!(
            event,
            BrowserEvent::LocationsRelocated { .. } | BrowserEvent::LocationsRemoved { .. }
        ) {
            observed.borrow_mut().push(event.clone());
        }
    });
    let folder = |path: &str| FileEntry {
        kind: EntryKind::Directory,
        ..fixture_entry(path)
    };

    browser.rename(folder("/fixture/photos"), "pictures".to_owned());
    browser.transfer(
        Location::local("/fixture/archive"),
        vec![PasteItem {
            source: Location::local("/fixture/pictures"),
            conflict: TransferConflict::FailIfExists,
        }],
        true,
        true,
    );
    browser.transfer(
        Location::local("/fixture/backup"),
        vec![PasteItem {
            source: Location::local("/fixture/archive/pictures"),
            conflict: TransferConflict::FailIfExists,
        }],
        false,
        true,
    );
    browser.delete(vec![folder("/fixture/archive/pictures")], false);

    let events = events.borrow();
    assert_eq!(events.len(), 3, "a copy leaves the source where it was");
    assert!(matches!(
        &events[0],
        BrowserEvent::LocationsRelocated { moves }
            if moves == &[(Location::local("/fixture/photos"), Location::local("/fixture/pictures"))]
    ));
    assert!(matches!(
        &events[1],
        BrowserEvent::LocationsRelocated { moves }
            if moves == &[(
                Location::local("/fixture/pictures"),
                Location::local("/fixture/archive/pictures")
            )]
    ));
    assert!(matches!(
        &events[2],
        BrowserEvent::LocationsRemoved { locations }
            if locations == &[Location::local("/fixture/archive/pictures")]
    ));
}
