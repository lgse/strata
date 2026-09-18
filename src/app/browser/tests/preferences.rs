// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn restored_sorting_applies_to_the_initial_navigation_load() {
    let browser = Browser::with_preferences(
        Rc::new(RestoredSortingSource),
        ViewPreferences {
            sort_key: SortKey::Size,
            sort_direction: SortDirection::Descending,
            ..ViewPreferences::default()
        },
    );

    browser.navigate(Location::local("/fixture"));

    let snapshot = browser.column_snapshot(0).expect("initial column");
    assert_eq!(snapshot.selected_positions, vec![0]);
    let names: Vec<_> = browser.state.borrow().columns[0]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["large".to_owned(), "small".to_owned()]);
    assert_eq!(
        browser
            .column_preferences(0)
            .expect("initial column preferences")
            .sort_key,
        SortKey::Size
    );
}

#[test]
fn hidden_file_preference_is_applied_to_reloaded_requests() {
    let request_count = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        request_count: request_count.clone(),
    }));
    let observed_preferences = Rc::new(Cell::new(None));
    let observed = observed_preferences.clone();
    browser.observe_preferences(move |preferences| observed.set(Some(preferences)));

    browser.navigate(Location::local("/fixture"));
    browser.toggle_hidden();

    // Toggling hidden files no longer re-enumerates; it only re-filters in-memory state.
    assert_eq!(request_count.get(), 1);
    assert_eq!(
        observed_preferences.get(),
        Some(ViewPreferences {
            show_hidden: true,
            ..ViewPreferences::default()
        })
    );
}

#[test]
fn new_columns_inherit_show_hidden_preference() {
    let preferences = ViewPreferences {
        show_hidden: true,
        ..Default::default()
    };
    let browser = Browser::with_preferences(Rc::new(FakeFileSource), preferences);
    browser.navigate(Location::local("/fixture"));

    assert_eq!(
        browser.column_preferences(0).map(|p| p.show_hidden),
        Some(true)
    );

    browser.descend(0, Location::local("/fixture/child"));
    assert_eq!(
        browser.column_preferences(1).map(|p| p.show_hidden),
        Some(true)
    );
}

#[test]
fn delayed_sort_cannot_restore_stale_hidden_file_preferences() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser
        .state
        .borrow_mut()
        .navigate(Location::local("/fixture"), RequestId(1));
    let pending = browser.preferences();
    browser.pending_sort.set(Some((1, 0)));
    browser.apply_default_preferences(ViewPreferences {
        show_hidden: true,
        ..pending
    });
    browser.finish_awaited_sort(0, 1, pending);
    assert!(browser.preferences().show_hidden);
    assert!(
        browser
            .column_preferences(0)
            .expect("existing column")
            .show_hidden
    );
}
