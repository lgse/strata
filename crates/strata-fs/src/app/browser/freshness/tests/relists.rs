// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_relist_waits_ten_times_its_own_duration() {
    let rows = [
        ("shown", false, false, vec!["added elsewhere", "listed"]),
        ("cut short", true, false, vec!["listed"]),
        ("failed", false, true, vec!["listed"]),
    ];
    assert_rows(rows, |(row, truncated, failed, shown)| {
        let source = Rc::new(Folders::default());
        let share = source.add("/net/share", network(1), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        browser.navigate(share.clone());
        source.hold_loads.set(true);
        source.truncate_loads.set(truncated);
        source.fail_loads.set(failed);
        source.change(&share, &["listed", "added elsewhere"]);

        clock.tick_at(15);
        clock.at(20);
        source.finish_held_loads();
        let relisted = (source.loads_of(&share), names_at(&browser, 0));
        clock.tick_at(69);
        let checks_within_ten_listings = source.checks_of(&share);
        clock.tick_at(70);

        let got = (
            relisted,
            checks_within_ten_listings,
            source.checks_of(&share),
        );
        mismatch(row, got, ((2, owned(&shown)), 1, 2))
    });
}

#[test]
fn a_background_relist_that_fails_keeps_the_previous_listing() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    let events = recorded(&browser);
    source.change(&share, &["listed", "added elsewhere"]);
    source.hold_loads.set(true);
    source.fail_loads.set(true);

    clock.tick_at(15);
    source.notify(&share, DirectoryChange::Upsert(entry(&share, "made here")));
    source.finish_held_loads();
    let kept = (
        names_at(&browser, 0),
        browser.state.borrow().columns[0].load_state.clone(),
    );
    source.hold_loads.set(false);
    source.fail_loads.set(false);
    clock.tick_at(30);

    assert_eq!(kept, (owned(&["listed", "made here"]), LoadState::Ready));
    let failures = count(&events, |event| {
        matches!(event, BrowserEvent::LoadFailed { .. })
    });
    let finishes = count(&events, |event| {
        matches!(event, BrowserEvent::LoadFinished { depth: 0, .. })
    });
    assert_eq!((failures, finishes), (0, 2));
    assert_eq!(source.loads_of(&share), 3);
    assert_eq!(names_at(&browser, 0), ["added elsewhere", "listed"]);
}

fn pump_until_settled(browser: &Browser) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !browser.settled(0) && Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_time_truncated_relist_does_not_replace_a_complete_listing() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let at_the_cap: Vec<String> = (0..crate::app::browser::MAX_DIRECTORY_ENTRIES)
        .map(|index| format!("entry {index}"))
        .collect();
    let rows = [
        ("complete before", false, 2, true),
        ("truncated before", true, 2, false),
        (
            "complete before, cut at the entry cap",
            false,
            at_the_cap.len(),
            false,
        ),
    ];
    assert_rows(rows, |(row, truncated_before, listed, kept)| {
        let source = Rc::new(Folders::default());
        let share = source.add("/net/share", network(1), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        source.truncate_loads.set(truncated_before);
        browser.navigate(share.clone());
        let names: Vec<&str> = match listed {
            2 => vec!["listed", "added elsewhere"],
            _ => at_the_cap.iter().map(String::as_str).collect(),
        };
        source.change(&share, &names);
        source.truncate_loads.set(true);

        clock.tick_at(15);
        pump_until_settled(&browser);

        let got = (
            names_at(&browser, 0).len(),
            browser.state.borrow().columns[0].truncated,
            source.validations.borrow().len(),
        );
        let expected = match kept {
            true => (1, false, 0),
            false => (listed, true, 0),
        };
        mismatch(row, got, expected)
    });
}

#[test]
fn a_refresh_on_a_mount_that_stopped_answering_keeps_its_rows() {
    let rows = [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
    ];
    assert_rows(
        rows,
        |(stalled, relisting_truncated, probe_runs_out_of_time)| {
            let source = Rc::new(Folders::default());
            let share = source.add("/net/share", network(1), Some(Watching::Live));
            let (browser, clock) = browser_on(&source);
            source.truncate_loads.set(relisting_truncated);
            source.probes_run_out_of_time.set(probe_runs_out_of_time);
            browser.navigate(share.clone());
            let events = recorded(&browser);
            if relisting_truncated {
                source.change(&share, &["listed", "added elsewhere"]);
                source.hold_loads.set(true);
                clock.tick_at(15);
            }
            if stalled {
                source.responsiveness.set(Responsiveness::Stalled);
            }

            browser.refresh_all();

            let got = (
                names_at(&browser, 0),
                count(&events, |event| {
                    matches!(event, BrowserEvent::ColumnReloaded { .. })
                }),
                count(&events, |event| {
                    matches!(event, BrowserEvent::LoadFailed { .. })
                }),
                browser.state.borrow().columns[0].truncated,
            );
            let expected = (
                owned(&["listed"]),
                usize::from(!stalled),
                0,
                relisting_truncated,
            );
            (got != expected).then(|| {
                format!(
                    "stalled {stalled}, re-listing a truncated listing {relisting_truncated}, \
                 the refresh running out of time {probe_runs_out_of_time}: \
                 got {got:?}, expected {expected:?}"
                )
            })
        },
    );
}

fn shown_rows(events: &RefCell<Vec<BrowserEvent>>) -> usize {
    events.borrow().iter().fold(0, |shown, event| match event {
        BrowserEvent::EntriesReplaced { depth: 0, count } => *count,
        BrowserEvent::EntriesPublished {
            depth: 0, count, ..
        } => shown + count,
        BrowserEvent::ColumnReloaded { depth: 0 } => 0,
        _ => shown,
    })
}

#[derive(Clone, Copy)]
enum KeptWhilePublishing {
    Refused,
    CutShort,
    FailedRelist,
}

struct Publishing {
    source: Rc<Folders>,
    share: Location,
    browser: Rc<Browser>,
    clock: Clock,
    names: Vec<String>,
}

impl Publishing {
    fn new() -> Self {
        let source = Rc::new(Folders::default());
        let share = source.add("/net/share", network(1), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        Self {
            source,
            share,
            browser,
            clock,
            names: (0..1000).map(|index| format!("entry {index:04}")).collect(),
        }
    }

    fn names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    fn pump_until_published(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.browser.staged_publishes.borrow().is_empty() && Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
        }
    }
}

impl KeptWhilePublishing {
    fn keep(self, publishing: &Publishing) {
        let Publishing {
            source,
            share,
            browser,
            clock,
            ..
        } = publishing;
        match self {
            Self::Refused | Self::CutShort => {
                source.change_names_only(share, &publishing.names());
                browser.navigate(share.clone());
                source
                    .probes_run_out_of_time
                    .set(matches!(self, Self::CutShort));
                source.responsiveness.set(Responsiveness::Stalled);
                browser.reload_active();
            }
            Self::FailedRelist => {
                browser.navigate(share.clone());
                source.change(share, &publishing.names());
                source.queue_loads.set(true);
                browser.reload_active();
                source.queue_loads.set(false);
                source.run_queued_loads();
                source.fail_loads.set(true);
                clock.tick_at(15);
            }
        }
    }

    fn loads(self) -> usize {
        match self {
            Self::FailedRelist => 3,
            _ => 2,
        }
    }
}

#[test]
fn a_refresh_kept_while_rows_are_still_publishing_shows_every_row() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let rows = [
        (
            "F5 turned away while the mount is stalled",
            KeptWhilePublishing::Refused,
        ),
        (
            "F5 cut short while the mount is stalled",
            KeptWhilePublishing::CutShort,
        ),
        (
            "a check's re-list that fails",
            KeptWhilePublishing::FailedRelist,
        ),
    ];
    assert_rows(rows, |(row, refresh)| {
        let publishing = Publishing::new();
        let events = recorded(&publishing.browser);
        refresh.keep(&publishing);
        publishing.pump_until_published();

        let rows = publishing.names.len();
        let got = (
            shown_rows(&events),
            publishing.browser.state.borrow().columns[0].entries.len(),
            count(&events, |event| {
                matches!(event, BrowserEvent::LoadFinished { depth: 0, .. })
            }),
            publishing.source.loads_of(&publishing.share),
        );
        let loads = refresh.loads();
        mismatch(row, got, (rows, rows, loads, loads))
    });
}
