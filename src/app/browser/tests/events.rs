// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn fan_out_shares_one_event_with_every_observer() {
    let browser = Browser::new(Rc::new(SortFillSource));
    let first = Rc::new(RefCell::new(Vec::new()));
    let second = Rc::new(RefCell::new(Vec::new()));
    let third = Rc::new(RefCell::new(Vec::new()));
    for collected in [first.clone(), second.clone(), third.clone()] {
        browser.observe(move |event| collected.borrow_mut().push(event.clone()));
    }

    browser.navigate(Location::local("/fixture"));
    for collected in [first.clone(), second.clone(), third.clone()] {
        assert!(
            collected
                .borrow()
                .iter()
                .any(|event| matches!(event, BrowserEvent::Reset))
        );
        let published: Vec<_> = collected
            .borrow()
            .iter()
            .filter_map(|event| match event {
                BrowserEvent::EntriesReplaced { count, .. } => Some(*count),
                _ => None,
            })
            .collect();
        assert_eq!(published, vec![2]);
    }
    assert_eq!(first.borrow().len(), second.borrow().len());
    assert_eq!(second.borrow().len(), third.borrow().len());
}

#[test]
fn observers_added_or_cleared_mid_dispatch_do_not_corrupt_it() {
    let browser = Browser::new(Rc::new(SortFillSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    let late = Rc::new(RefCell::new(Vec::new()));
    let late_events = late.clone();
    let browser_for_observer = browser.clone();
    browser.observe(move |event| {
        observed.borrow_mut().push(event.clone());
        let value = late_events.clone();
        browser_for_observer.observe(move |event| {
            value.borrow_mut().push(event.clone());
        });
        browser_for_observer.clear_observer();
    });

    browser.navigate(Location::local("/fixture"));
    let first_wave = events.borrow().len();
    assert!(first_wave > 0);
    assert!(late.borrow().is_empty());

    browser.navigate(Location::local("/elsewhere"));
    assert_eq!(events.borrow().len(), first_wave);
}

#[test]
fn nested_emission_during_dispatch_is_safe() {
    let browser = Browser::new(Rc::new(SortFillSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    let browser_for_observer = browser.clone();
    browser.observe(move |event| {
        let select_now =
            matches!(event, BrowserEvent::EntriesReplaced { .. }) && observed.borrow().len() < 4;
        observed.borrow_mut().push(event.clone());
        if select_now {
            browser_for_observer.select(0, 0);
        }
    });

    browser.navigate(Location::local("/fixture"));
    assert!(
        events.borrow().iter().any(|event| {
            matches!(
                event,
                BrowserEvent::FocusChanged {
                    position: Some(0),
                    ..
                }
            )
        }),
        "the nested select should have been dispatched"
    );
}
