// SPDX-License-Identifier: MIT

use super::*;

type Validation = Rc<dyn Fn(Result<(), LocationValidationError>)>;

#[derive(Default)]
struct LifecycleSource {
    callbacks: RefCell<Vec<(Location, Validation)>>,
    immediate: Option<Location>,
    cancelled: Rc<RefCell<Vec<Location>>>,
}

impl FileSource for LifecycleSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn validate_location_async(&self, location: Location, emit: Validation) -> LoadHandle {
        if self.immediate.as_ref() == Some(&location) {
            emit(Ok(()));
        } else {
            self.callbacks.borrow_mut().push((location.clone(), emit));
        }
        let cancelled = self.cancelled.clone();
        LoadHandle::new(move || cancelled.borrow_mut().push(location))
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
}

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

#[test]
fn navigation_observers_can_reenter_and_add_observers_without_losing_newer_validation() {
    let source = Rc::new(LifecycleSource::default());
    let browser = Browser::new(source.clone());
    browser.navigate(Location::local("/fixture"));
    let first = Location::uri("sftp://fixture/first");
    let newer = Location::uri("sftp://fixture/newer");
    let started = Rc::new(Cell::new(false));
    let start = started.clone();
    let reentrant = browser.clone();
    let target = newer.clone();
    let snapshots = Rc::new(RefCell::new(Vec::new()));
    let late_snapshots = snapshots.clone();
    browser.observe_navigation(move || {
        if reentrant.pending_navigation_generation().is_some() && !start.replace(true) {
            let observed = late_snapshots.clone();
            let weak = Rc::downgrade(&reentrant);
            reentrant.observe_navigation(move || {
                if let Some(browser) = weak.upgrade() {
                    observed
                        .borrow_mut()
                        .push(browser.pending_navigation_generation());
                }
            });
            reentrant.navigate_validated(target.clone(), true);
        }
    });
    browser.navigate_validated(first.clone(), true);
    assert!(source.cancelled.borrow().contains(&first));
    assert!(!source.cancelled.borrow().contains(&newer));
    let older = source.callbacks.borrow_mut().remove(0).1;
    older(Ok(()));
    assert!(browser.pending_navigation_generation().is_some());
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    let complete = source.callbacks.borrow_mut().remove(0).1;
    complete(Ok(()));
    assert_eq!(browser.active_location(), Some(newer));
    assert_eq!(browser.pending_navigation_generation(), None);
    assert_eq!(snapshots.borrow().last(), Some(&None));
    browser.clear_observer();
    let count = snapshots.borrow().len();
    browser.navigate(Location::local("/elsewhere"));
    assert_eq!(snapshots.borrow().len(), count);
}

#[test]
fn synchronous_descent_does_not_overwrite_validation_started_by_an_observer() {
    let alpha = Location::uri("sftp://fixture/alpha");
    let source = Rc::new(LifecycleSource {
        immediate: Some(alpha.clone()),
        ..LifecycleSource::default()
    });
    let browser = Browser::new(source.clone());
    browser.navigate(Location::local("/fixture"));
    let newer = Location::uri("sftp://fixture/newer");
    let reentrant = browser.clone();
    let target = newer.clone();
    let first = alpha.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::ColumnAdded { depth: 1, location } if location == &first) {
            reentrant.navigate_validated(target.clone(), true);
        }
    });
    browser.descend(0, alpha);
    assert!(browser.pending_navigation_generation().is_some());
    assert!(!source.cancelled.borrow().contains(&newer));
    let complete = source.callbacks.borrow_mut().remove(0).1;
    complete(Ok(()));
    assert_eq!(browser.active_location(), Some(newer));
    assert_eq!(browser.pending_navigation_generation(), None);
    browser.clear_observer();
}
