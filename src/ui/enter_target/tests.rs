// SPDX-License-Identifier: MIT
use super::*;

enum Step {
    Show(&'static str),
    Edit,
    Restart,
    Pick,
    Scroll,
}

struct Case {
    name: &'static str,
    tenxer: bool,
    steps: &'static [Step],
    last_show_changed: bool,
    offers_top: bool,
    follows_top: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "a first query offers and follows the top",
        tenxer: false,
        steps: &[Step::Show("rep")],
        last_show_changed: true,
        offers_top: true,
        follows_top: true,
    },
    Case {
        name: "a later batch for the same query keeps the pick",
        tenxer: false,
        steps: &[Step::Show("rep"), Step::Pick, Step::Show("rep")],
        last_show_changed: false,
        offers_top: false,
        follows_top: false,
    },
    Case {
        name: "a new query text drops the pick",
        tenxer: false,
        steps: &[
            Step::Show("rep"),
            Step::Pick,
            Step::Edit,
            Step::Show("repo"),
        ],
        last_show_changed: true,
        offers_top: true,
        follows_top: true,
    },
    Case {
        name: "an edit typed back to the same text still drops the pick",
        tenxer: false,
        steps: &[Step::Show("rep"), Step::Pick, Step::Edit, Step::Show("rep")],
        last_show_changed: true,
        offers_top: true,
        follows_top: true,
    },
    Case {
        name: "a restarted query counts as new",
        tenxer: false,
        steps: &[
            Step::Show("rep"),
            Step::Pick,
            Step::Restart,
            Step::Show("rep"),
        ],
        last_show_changed: true,
        offers_top: true,
        follows_top: true,
    },
    Case {
        name: "scrolling the results stops following the top but Enter still opens it",
        tenxer: false,
        steps: &[Step::Show("rep"), Step::Scroll, Step::Show("rep")],
        last_show_changed: false,
        offers_top: true,
        follows_top: false,
    },
    Case {
        name: "a new query follows the top again after scrolling",
        tenxer: false,
        steps: &[
            Step::Show("rep"),
            Step::Scroll,
            Step::Edit,
            Step::Show("repo"),
        ],
        last_show_changed: true,
        offers_top: true,
        follows_top: true,
    },
    Case {
        name: "an empty query offers nothing",
        tenxer: false,
        steps: &[Step::Show("rep"), Step::Edit, Step::Show("  ")],
        last_show_changed: false,
        offers_top: false,
        follows_top: false,
    },
    Case {
        name: "10xer never resets a pick or offers the top",
        tenxer: true,
        steps: &[
            Step::Show("rep"),
            Step::Pick,
            Step::Edit,
            Step::Show("repo"),
        ],
        last_show_changed: false,
        offers_top: false,
        follows_top: false,
    },
];

#[test]
fn query_changes_reset_the_pick_and_the_scroll_follow() {
    for case in CASES {
        let mut tracker = Tracker::default();
        let mut changed = false;
        for step in case.steps {
            match step {
                Step::Show(query) => changed = tracker.show_query(query, case.tenxer),
                Step::Edit => tracker.edited = true,
                Step::Restart => tracker.query = None,
                Step::Pick => tracker.picked = true,
                Step::Scroll => tracker.scrolled = true,
            }
        }
        assert_eq!(changed, case.last_show_changed, "{}: changed", case.name);
        assert_eq!(
            tracker.offers_top(true, true, case.tenxer),
            case.offers_top,
            "{}: offers top",
            case.name
        );
        assert_eq!(
            tracker.follows_top(true, true, case.tenxer),
            case.follows_top,
            "{}: follows top",
            case.name
        );
    }
}

#[test]
fn a_pick_counts_only_while_something_is_selected() {
    let mut tracker = Tracker::default();
    tracker.show_query("rep", false);
    tracker.picked = true;
    assert!(!tracker.offers_top(true, true, false));
    assert!(tracker.offers_top(true, false, false));
    assert!(!tracker.offers_top(false, false, false));
}
