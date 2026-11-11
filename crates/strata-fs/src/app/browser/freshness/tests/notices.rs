// SPDX-License-Identifier: MIT

use super::*;

fn notices(events: &RefCell<Vec<BrowserEvent>>) -> Vec<(usize, bool)> {
    events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::ColumnNotResponding {
                depth,
                not_responding,
            } => Some((*depth, *not_responding)),
            _ => None,
        })
        .collect()
}

fn notice_shown(browser: &Browser, depth: usize) -> bool {
    browser
        .column_snapshot(depth)
        .is_some_and(|column| column.not_responding)
}

#[test]
fn a_mount_that_stops_answering_shows_the_notice_until_it_answers() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    let events = recorded(&browser);

    source.responsiveness.set(Responsiveness::Stalled);
    clock.tick_at(15);
    let stalled = (notices(&events), browser.freshness.following.get());
    source.responsiveness.set(Responsiveness::NotResponding);
    browser.follow_responsiveness();
    let not_responding = (
        notices(&events),
        notice_shown(&browser, 0),
        names_at(&browser, 0),
        source.loads_of(&share),
    );
    source.responsiveness.set(Responsiveness::Answering);
    browser.follow_responsiveness();

    assert_eq!(stalled, (vec![], true));
    assert_eq!(
        not_responding,
        (vec![(0, true)], true, owned(&["listed"]), 1)
    );
    let answered = (
        notices(&events),
        notice_shown(&browser, 0),
        browser.freshness.following.get(),
    );
    assert_eq!(answered, (vec![(0, true), (0, false)], false, false));
}

#[test]
fn a_mount_that_stops_answering_a_request_from_elsewhere_shows_the_notice() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, _clock) = browser_on(&source);
    browser.set_freshness(FreshnessInput {
        presence: Presence::Focused,
        watch_network_folders: false,
        ..browser.freshness_input()
    });
    browser.navigate(share);
    let idle = (
        browser.freshness.armed_for.get(),
        browser.freshness.following.get(),
    );

    source.responsiveness.set(Responsiveness::NotResponding);
    let followers = source.followers.borrow().clone();
    for notify in followers {
        notify();
    }

    assert_eq!(idle, (None, false));
    assert!(notice_shown(&browser, 0));
}

#[test]
fn the_notice_stays_off_a_folder_whose_load_failed() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let sub = source.add("/net/share/sub", network(2), Some(Watching::Live));
    let (browser, _clock) = browser_on(&source);
    browser.navigate(share);
    source.fail_loads.set(true);
    browser.show_child(0, sub);
    let events = recorded(&browser);

    source.responsiveness.set(Responsiveness::NotResponding);
    browser.follow_responsiveness();

    assert_eq!(notices(&events), [(0, true)]);
    assert!(!notice_shown(&browser, 1));
}

#[test]
fn responsiveness_is_followed_only_while_visible_and_a_mount_is_busy_or_stalled() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    let following = || browser.freshness.following.get();
    let idle = following();
    source.hold_checks.set(true);
    clock.tick_at(15);
    let checking = following();
    source.answer_held_checks();
    let answered = following();
    source.responsiveness.set(Responsiveness::Stalled);
    browser.follow_responsiveness();
    let stalled = following();
    let input = browser.freshness_input();
    browser.set_freshness(FreshnessInput {
        presence: Presence::Hidden,
        ..input
    });
    let hidden = following();
    browser.set_freshness(input);

    assert_eq!(
        (idle, checking, answered, stalled, hidden),
        (false, true, false, true, false)
    );
    assert!(following());
}
