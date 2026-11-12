// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn closing_a_sorting_column_allows_observers_to_select_in_the_parent() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, events, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.descend(0, Location::local("/fixture/sub"));
    browser.set_sort(1, SortKey::Size, SortDirection::Ascending);
    pump_until(|| !source.fill_calls.borrow().is_empty());

    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::SortingFinished { depth: 1 }) {
            weak.upgrade().expect("live browser").select(0, 1);
        }
    });
    browser.close_column(1);

    assert_eq!(finish_count(&events), 1);
    let (depth, position, entry) = browser.focused_item().expect("parent selection");
    assert_eq!((depth, position), (0, 1));
    assert_eq!(entry.display_name, "beta");
}

#[test]
fn closing_a_column_keeps_only_the_parent_viewport_fill() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let (browser, _, source) = scripted_browser(ScriptedSource::scripted(
        vec!["alpha", "beta"],
        vec![FillAnswer::Never],
    ));
    browser.navigate(Location::local("/fixture"));
    browser.descend(0, Location::local("/fixture/sub"));
    browser.request_metadata_fill(0, 0, Location::local("/fixture/alpha"), false);
    browser.request_metadata_fill(1, 1, Location::local("/fixture/beta"), false);

    browser.close_column(1);
    pump_until(|| !source.fill_calls.borrow().is_empty());

    let calls = source.fill_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].full);
    assert_eq!(calls[0].entries, vec![Location::local("/fixture/alpha")]);
}
