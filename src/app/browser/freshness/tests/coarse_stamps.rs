// SPDX-License-Identifier: MIT

use super::*;

fn coarse(
    modified: (i64, u32),
    read_at_millis: u64,
    coarse_window: Option<u64>,
    daemon_cache: Option<u64>,
) -> DirectoryStamp {
    DirectoryStamp {
        modified,
        changed: Some(modified),
        coarse_window: coarse_window.map(Duration::from_secs),
        daemon_cache: daemon_cache.map(Duration::from_secs),
        read_at: UNIX_EPOCH + Duration::from_millis(read_at_millis),
        ..network(1)
    }
}

fn first_relist_and_last_loads(loads: &[usize]) -> (Option<u64>, Option<usize>) {
    let first_relist = loads
        .iter()
        .position(|loads| *loads > 1)
        .map(|index| index as u64 + 1);
    (first_relist, loads.last().copied())
}

#[test]
fn a_coarse_baseline_is_confirmed_once_after_its_window() {
    let rows = [
        (
            "whole seconds",
            coarse((1_000, 0), 1_000_500, Some(2), None),
            false,
            Some(2),
        ),
        (
            "whole seconds whose confirmation fails once",
            coarse((1_000, 0), 1_000_500, Some(2), None),
            true,
            Some(2),
        ),
        (
            "whole seconds behind a daemon's cache",
            coarse((1_000, 0), 1_000_500, Some(3), Some(21)),
            false,
            Some(21),
        ),
        (
            "whole seconds read after their window",
            coarse((1_000, 0), 1_005_000, Some(2), None),
            false,
            None,
        ),
        (
            "coarse ticks with fractions",
            coarse((1_000, 10_000_000), 1_000_500, Some(2), None),
            false,
            None,
        ),
        (
            "fine-grained",
            coarse((1_000, 5), 1_000_500, None, None),
            false,
            None,
        ),
    ];
    assert_rows(rows, |(row, stamp, fails_once, confirmed_at)| {
        let source = Rc::new(Folders::default());
        let share = source.add("/net/share", stamp, Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        browser.navigate(share.clone());
        source.fail_loads.set(fails_once);

        let loads: Vec<usize> = (1..=40)
            .map(|second| {
                clock.tick_at(second);
                if source.loads_of(&share) > 1 {
                    source.fail_loads.set(false);
                }
                source.loads_of(&share)
            })
            .collect();

        let expected = (
            confirmed_at,
            Some(1 + usize::from(confirmed_at.is_some()) + usize::from(fails_once)),
        );
        mismatch(row, first_relist_and_last_loads(&loads), expected)
    });
}

#[test]
fn a_confirmation_and_a_check_due_together_on_one_mount_both_go_through() {
    let source = Rc::new(Folders::default());
    let share = source.add(
        "/net/share",
        coarse((1_000, 0), 1_000_500, Some(3), Some(15)),
        Some(Watching::Live),
    );
    let sub = source.add("/net/share/sub", network(2), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    browser.show_child(0, sub.clone());
    source.change(&sub, &["listed", "added elsewhere"]);
    source.hold_checks.set(true);
    source.hold_loads.set(true);

    clock.tick_at(15);
    source.answer_held_checks();
    source.finish_held_loads();
    source.finish_held_loads();

    assert_eq!(source.loads_of(&share), 2);
    assert_eq!(names_at(&browser, 1), ["added elsewhere", "listed"]);
}

#[derive(Clone, Copy)]
enum Elsewhere {
    Moved(&'static [&'static str]),
    Names(&'static [&'static str]),
    Refresh,
    Tick(u64),
}

const ADDED: &[&str] = &["listed", "added elsewhere"];

/// A listing changed elsewhere, seen through a stamp that may not have moved.
struct EqualStamp {
    mount: u64,
    case: &'static str,
    baseline: DirectoryStamp,
    before: &'static [Elsewhere],
    /// Checks and re-lists at 3 s, checks and re-lists at 15 s, whether the late name
    /// shows, and checks of the folder on another mount.
    expected: (usize, usize, usize, usize, bool, usize),
}

fn equal_stamps() -> [EqualStamp; 5] {
    use Elsewhere::{Moved, Names, Refresh, Tick};
    [
        EqualStamp {
            mount: 50,
            case: "names changed under an equal stamp",
            baseline: network(1),
            before: &[Names(ADDED), Refresh],
            expected: (0, 0, 0, 1, true, 2),
        },
        EqualStamp {
            mount: 51,
            case: "the stamp moved and the names stayed",
            baseline: network(1),
            before: &[Moved(&["listed"]), Refresh],
            expected: (1, 0, 2, 0, false, 2),
        },
        EqualStamp {
            mount: 52,
            case: "names changed under a coarse stamp read within its window",
            baseline: coarse((1_000, 0), 1_000_500, Some(2), None),
            before: &[Names(ADDED), Refresh],
            expected: (1, 1, 2, 1, true, 2),
        },
        EqualStamp {
            mount: 53,
            case: "names a client cache kept under a moved stamp changed later",
            baseline: network(1),
            before: &[Moved(&["listed"]), Tick(3), Refresh, Names(ADDED), Refresh],
            expected: (2, 0, 3, 0, false, 3),
        },
        EqualStamp {
            mount: 54,
            case: "names a daemon cache refreshed before the stamp",
            baseline: coarse((1_000, 0), 1_030_000, Some(1), Some(21)),
            before: &[Names(ADDED), Refresh],
            expected: (1, 0, 2, 0, false, 2),
        },
    ]
}

struct BlindShare {
    source: Rc<Folders>,
    share: Location,
    other: Location,
    browser: Rc<Browser>,
    clock: Clock,
}

impl BlindShare {
    fn open(mount: u64, baseline: DirectoryStamp) -> Self {
        let source = Rc::new(Folders::default());
        let baseline = DirectoryStamp {
            mount: Some(MountToken::Native(mount)),
            ..baseline
        };
        let share = source.add("/net/blind", baseline, Some(Watching::Live));
        let other = source.add("/net/blind/other", stamp(NFS + 1, 2, true), None);
        let (browser, clock) = browser_on(&source);
        set_presence(&browser, Presence::Focused);
        browser.navigate(share.clone());
        browser.show_child(0, other.clone());
        Self {
            source,
            share,
            other,
            browser,
            clock,
        }
    }

    /// Returns the second the last tick ran at.
    fn replay(&self, steps: &[Elsewhere]) -> u64 {
        let mut now = 0;
        for step in steps {
            match *step {
                Elsewhere::Moved(names) => self.source.change(&self.share, names),
                Elsewhere::Names(names) => self.source.change_names_only(&self.share, names),
                Elsewhere::Refresh => self.browser.refresh_all(),
                Elsewhere::Tick(second) => {
                    now = second;
                    self.clock.tick_at(second);
                }
            }
        }
        now
    }
}

impl EqualStamp {
    fn check(self) -> Option<String> {
        let blind = BlindShare::open(self.mount, self.baseline);
        let BlindShare {
            source,
            share,
            other,
            browser,
            clock,
        } = &blind;
        let now = blind.replay(self.before);
        let loads = source.loads_of(share);
        source.change_names_only(share, &["listed", "added elsewhere", "added later"]);

        clock.tick_at(now + 3);
        let at_three = (source.checks_of(share), source.loads_of(share) - loads);
        clock.tick_at(now + 15);

        let got = (
            at_three.0,
            at_three.1,
            source.checks_of(share),
            source.loads_of(share) - loads,
            names_at(browser, 0).contains(&"added later".to_owned()),
            source.checks_of(other),
        );
        mismatch(self.case, got, self.expected)
    }
}

#[test]
fn a_listing_that_changed_under_an_equal_stamp_switches_its_mount_to_relisting() {
    assert_rows(equal_stamps(), EqualStamp::check);
}

#[test]
fn a_relisted_mount_slows_down_until_its_listing_changes() {
    let source = Rc::new(Folders::default());
    let blind = DirectoryStamp {
        mount: Some(MountToken::Native(60)),
        ..network(1)
    };
    let share = source.add("/net/blind", blind, Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    source.change_names_only(&share, &["listed", "added elsewhere"]);
    browser.refresh_all();
    let learned = source.loads_of(&share);
    let relists = || source.loads_of(&share) - learned;

    for second in (15..=300).step_by(15) {
        clock.tick_at(second);
    }
    let after_twenty_unchanged = relists();
    clock.tick_at(315);
    let at_the_usual_interval = relists();
    clock.tick_at(330);
    let at_twice_the_interval = relists();
    source.change_names_only(&share, &["listed", "added elsewhere", "added later"]);
    clock.tick_at(360);
    clock.tick_at(375);

    assert_eq!(
        (
            after_twenty_unchanged,
            at_the_usual_interval,
            at_twice_the_interval
        ),
        (20, 20, 21)
    );
    assert_eq!(relists(), 23);
    assert_eq!(source.checks_of(&share), 0);
}
