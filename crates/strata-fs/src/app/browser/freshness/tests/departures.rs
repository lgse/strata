// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_folder_found_inaccessible_shows_the_failure_and_is_no_longer_checked() {
    let source = Rc::new(Folders::default());
    let home = source.add("/home/me", local(1), Some(Watching::Live));
    let private = source.add("/home/me/nas", network(2), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(home.clone());
    browser.show_child(0, private.clone());
    let events = recorded(&browser);
    source.answer(&private, StampAnswer::Inaccessible);

    clock.tick_at(15);
    let armed_after_the_failure = clock.armed();
    clock.tick_at(30);

    assert_eq!(
        count(&events, |event| matches!(
            event,
            BrowserEvent::LoadFailed { depth: 1, .. }
        )),
        1
    );
    assert_eq!(armed_after_the_failure, None);
    assert_eq!(source.checks_of(&private), 1);
}

fn moved(answer: StampAnswer) -> StampAnswer {
    match answer {
        StampAnswer::Stamp(mut stamp) => {
            stamp.modified.0 += 1;
            StampAnswer::Stamp(stamp)
        }
        other => other,
    }
}

#[test]
fn a_check_finding_the_folder_gone_runs_the_departure_check_under_a_watching_parent() {
    let rows = [
        (Some(Watching::Live), false),
        (None, false),
        (Some(Watching::Live), true),
        (None, true),
    ];
    assert_rows(rows, |(parent, found_by_a_relist)| {
        let source = Rc::new(Folders::default());
        let share = source.add("/net/share", network(1), parent);
        let sub = source.add("/net/share/sub", network(2), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        browser.navigate(share.clone());
        browser.show_child(0, sub.clone());
        source.hold_checks.set(true);

        clock.tick_at(15);
        let moved = moved(source.answer_of(&sub));
        source.remove(&sub);
        match found_by_a_relist {
            true => source.answer_held_checks_for(&sub, &moved),
            false => source.answer_held_checks(),
        }

        let got = (browser.location_at(0), browser.location_at(1));
        (got != (Some(share.clone()), None)).then(|| {
            format!("parent monitor {parent:?}, found by a re-list {found_by_a_relist}: {got:?}")
        })
    });
}

#[test]
fn a_folder_removed_with_its_open_subfolder_leaves_for_the_folder_above_both() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let outer = source.add("/net/share/outer", network(2), Some(Watching::Live));
    let inner = source.add("/net/share/outer/inner", network(3), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    browser.show_child(0, outer.clone());
    browser.show_child(1, inner.clone());
    source.remove(&outer);
    source.remove(&inner);
    source.hold_validations.set(true);

    clock.tick_at(15);
    source.hold_validations.set(false);
    source.answer_held_validations();

    assert_eq!(
        (browser.location_at(0), browser.location_at(1)),
        (Some(share), None)
    );
}

#[test]
fn a_failed_relist_leaves_a_departure_check_in_flight_alone() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let sub = source.add(
        "/net/share/sub",
        stamp(NFS + 1, 2, true),
        Some(Watching::Live),
    );
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    browser.show_child(0, sub.clone());
    source.change(&share, &["listed", "added elsewhere"]);
    source.hold_checks.set(true);
    source.hold_validations.set(true);
    source.fail_loads.set(true);

    clock.tick_at(15);
    source.remove(&sub);
    source.answer_held_check(&sub);
    source.answer_held_check(&share);
    source.hold_validations.set(false);
    source.answer_held_validations();

    assert_eq!(
        (browser.location_at(0), browser.location_at(1)),
        (Some(share), None)
    );
}

#[test]
fn a_check_finding_the_folder_replaced_reloads_it_in_place() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    source.replace(&share, 9, &["made anew"]);

    clock.tick_at(15);

    assert_eq!(browser.location_at(0), Some(share.clone()));
    assert_eq!(names_at(&browser, 0), ["made anew"]);
    assert_eq!(
        (source.loads_of(&share), source.monitors_requested(&share)),
        (2, 2)
    );
}

#[test]
fn an_unreachable_folder_keeps_its_listing_and_is_retried_later() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    source.hold_checks.set(true);

    clock.tick_at(15);
    source.answer_held_checks_for(&share, &StampAnswer::Unreachable);
    clock.tick_at(30);
    let checked_at_the_usual_interval = source.checks_of(&share);
    clock.tick_at(45);

    assert_eq!(
        (checked_at_the_usual_interval, source.checks_of(&share)),
        (1, 2)
    );
    let shown = (
        browser.location_at(0),
        names_at(&browser, 0),
        browser.state.borrow().columns[0].load_state.clone(),
    );
    assert_eq!(shown, (Some(share), owned(&["listed"]), LoadState::Ready));
}
