// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn checks_stop_while_hidden_and_resume_on_focus() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    set_presence(&browser, Presence::Hidden);
    let armed_while_hidden = clock.armed();

    clock.tick_at(15);
    let checked_while_hidden = source.checks_of(&share);
    set_presence(&browser, Presence::Focused);
    let checked_on_focus = source.checks_of(&share);
    clock.tick_at(17);
    let checked_before_the_focused_interval = source.checks_of(&share);
    clock.tick_at(18);

    assert_eq!(armed_while_hidden, None);
    assert_eq!(
        (
            checked_while_hidden,
            checked_on_focus,
            checked_before_the_focused_interval
        ),
        (0, 1, 1)
    );
    assert_eq!(source.checks_of(&share), 2);
}

#[test]
fn power_saver_slows_focused_checks_and_stops_unfocused_ones() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.set_freshness(FreshnessInput {
        presence: Presence::Focused,
        power_saver: true,
        ..browser.freshness_input()
    });
    browser.navigate(share.clone());
    let armed_focused = clock.armed();

    clock.tick_at(11);
    let before_four_intervals = source.checks_of(&share);
    clock.tick_at(12);
    let at_four_intervals = source.checks_of(&share);
    set_presence(&browser, Presence::Visible);
    let armed_unfocused = clock.armed();
    clock.tick_at(60);
    let unfocused = source.checks_of(&share);
    browser.set_freshness(FreshnessInput {
        power_saver: false,
        ..browser.freshness_input()
    });
    clock.tick_at(61);

    assert_eq!(
        (
            armed_focused,
            (before_four_intervals, at_four_intervals),
            (armed_unfocused, unfocused)
        ),
        (Some(Duration::from_secs(12)), (0, 1), (None, 1))
    );
    assert_eq!(source.checks_of(&share), 2);
}

#[derive(Clone, Copy)]
enum Blocker {
    InlineEdit,
    Operation,
}

/// Leaves the window with a checked network folder open, then focuses it again.
struct FocusReturn {
    row: &'static str,
    away: u64,
    asleep: u64,
    blocker: Option<Blocker>,
    /// Checks at focus, network loads at focus, network loads a minute later.
    expected: (usize, usize, usize),
}

const FOCUS_RETURNS: [FocusReturn; 5] = [
    FocusReturn {
        row: "away 29 s",
        away: 29,
        asleep: 0,
        blocker: None,
        expected: (1, 1, 1),
    },
    FocusReturn {
        row: "away 30 s",
        away: 30,
        asleep: 0,
        blocker: None,
        expected: (0, 2, 2),
    },
    FocusReturn {
        row: "away 20 s around 8 h asleep",
        away: 20,
        asleep: 8 * 3_600,
        blocker: None,
        expected: (0, 2, 2),
    },
    FocusReturn {
        row: "an inline edit open at focus",
        away: 40,
        asleep: 0,
        blocker: Some(Blocker::InlineEdit),
        expected: (1, 1, 2),
    },
    FocusReturn {
        row: "an operation writing into the folder at focus",
        away: 40,
        asleep: 0,
        blocker: Some(Blocker::Operation),
        expected: (0, 1, 2),
    },
];

/// A local folder with a checked network folder open beside it, after leaving the window.
struct AwayFixture {
    source: Rc<Folders>,
    home: Location,
    nas: Location,
    browser: Rc<Browser>,
    clock: Clock,
    editing: Rc<Cell<bool>>,
}

impl AwayFixture {
    fn left_at_one_second() -> Self {
        let source = Rc::new(Folders::default());
        let home = source.add("/freshness/focus", local(1), Some(Watching::Live));
        let nas = source.add("/freshness/focus/nas", network(2), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        let editing = hold_freshness(&browser, false);
        set_presence(&browser, Presence::Focused);
        browser.navigate(home.clone());
        browser.show_child(0, nas.clone());
        clock.at(1);
        set_presence(&browser, Presence::Visible);
        Self {
            source,
            home,
            nas,
            browser,
            clock,
            editing,
        }
    }

    fn focus_while_blocked(&self, blocker: Option<Blocker>) -> (usize, usize) {
        self.editing
            .set(matches!(blocker, Some(Blocker::InlineEdit)));
        let activity = matches!(blocker, Some(Blocker::Operation)).then(|| writing_into(&self.nas));
        set_presence(&self.browser, Presence::Focused);
        let at_focus = (
            self.source.checks_of(&self.nas),
            self.source.loads_of(&self.nas),
        );
        self.editing.set(false);
        drop(activity);
        at_focus
    }
}

impl FocusReturn {
    fn check(&self) -> Option<String> {
        let fixture = AwayFixture::left_at_one_second();
        fixture.clock.sleep(self.asleep);

        fixture.clock.at(1 + self.away);
        let at_focus = fixture.focus_while_blocked(self.blocker);
        for second in 2 + self.away..=61 + self.away {
            fixture.clock.tick_at(second);
        }

        let got = (
            at_focus.0,
            at_focus.1,
            fixture.source.loads_of(&fixture.nas),
        );
        let home_loads = fixture.source.loads_of(&fixture.home);
        ((got, home_loads) != (self.expected, 1)).then(|| {
            format!(
                "{}: got (checks at focus, network loads at focus, network loads a minute \
                 later) {got:?} and {home_loads} home loads, expected {:?} and 1",
                self.row, self.expected
            )
        })
    }
}

#[test]
fn focus_after_thirty_seconds_relists_visible_checked_folders() {
    assert_rows(&FOCUS_RETURNS, FocusReturn::check);
}

/// Leaves the window while a re-list runs, then comes back before or after it ends.
struct DutyCycle {
    row: &'static str,
    finished: u64,
    left: u64,
    back: u64,
    changed: bool,
    floor: u64,
    checks: usize,
}

const DUTY_CYCLES: [DutyCycle; 3] = [
    DutyCycle {
        row: "back after 1 s to a change",
        finished: 13,
        left: 14,
        back: 15,
        changed: true,
        floor: 113,
        checks: 2,
    },
    DutyCycle {
        row: "back after 30 s",
        finished: 13,
        left: 14,
        back: 44,
        changed: false,
        floor: 113,
        checks: 2,
    },
    DutyCycle {
        row: "back after 30 s while the re-list runs",
        finished: 36,
        left: 4,
        back: 34,
        changed: false,
        floor: 366,
        checks: 1,
    },
];

struct DutyFixture {
    source: Rc<Folders>,
    share: Location,
    browser: Rc<Browser>,
    clock: Clock,
}

impl DutyFixture {
    fn relisting() -> Self {
        let source = Rc::new(Folders::default());
        let share = source.add("/freshness/duty", network(1), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        set_presence(&browser, Presence::Focused);
        browser.navigate(share.clone());
        source.change(&share, &["listed", "a"]);
        source.hold_loads.set(true);
        clock.tick_at(3);
        Self {
            source,
            share,
            browser,
            clock,
        }
    }

    fn finish_at(&self, second: u64) {
        self.clock.at(second);
        self.source.finish_held_loads();
        self.source.hold_loads.set(false);
    }

    fn loads_and_checks(&self) -> (usize, usize) {
        (
            self.source.loads_of(&self.share),
            self.source.checks_of(&self.share),
        )
    }
}

impl DutyCycle {
    fn check(&self) -> Option<String> {
        let fixture = DutyFixture::relisting();
        self.leave(&fixture);

        fixture.clock.at(self.back);
        set_presence(&fixture.browser, Presence::Focused);
        let on_return = fixture.source.loads_of(&fixture.share);
        if self.finished > self.back {
            fixture.finish_at(self.finished);
        }
        for second in self.back.max(self.finished) + 1..self.floor {
            fixture.clock.tick_at(second);
        }
        let before_the_floor = fixture.loads_and_checks();
        fixture.clock.tick_at(self.floor);

        let got = (
            on_return,
            before_the_floor.0,
            fixture.source.loads_of(&fixture.share),
            before_the_floor.1,
        );
        let expected = (2, 2, 3, self.checks);
        (got != expected).then(|| {
            format!(
                "{}: got (loads on return, before {floor} s, at {floor} s, checks before \
                 {floor} s) {got:?}, expected {expected:?}",
                self.row,
                floor = self.floor
            )
        })
    }

    fn leave(&self, fixture: &DutyFixture) {
        if self.finished < self.left {
            fixture.finish_at(self.finished);
        }
        if self.changed {
            fixture.source.change(&fixture.share, &["listed", "a", "b"]);
        }
        fixture.clock.at(self.left);
        set_presence(&fixture.browser, Presence::Visible);
    }
}

#[test]
fn a_focus_relist_waits_for_the_relist_duty_cycle() {
    assert_rows(&DUTY_CYCLES, DutyCycle::check);
}

#[test]
fn a_stale_folder_waits_for_an_open_inline_edit() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    let editing = hold_freshness(&browser, true);
    browser.navigate(share.clone());
    source.change(&share, &["listed", "added elsewhere"]);

    clock.tick_at(15);
    clock.tick_at(30);
    let while_editing = (source.checks_of(&share), source.loads_of(&share));
    editing.set(false);
    clock.tick_at(45);

    assert_eq!(while_editing, (2, 1));
    assert_eq!((source.checks_of(&share), source.loads_of(&share)), (3, 2));
}
