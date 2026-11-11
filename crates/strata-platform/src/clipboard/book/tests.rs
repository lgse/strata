use super::Book;
use crate::clipboard::api::{Change, CopyId, CopyState, Lost, Offered, Selection};

const C: Selection = Selection::Clipboard;

fn m(types: &[&str]) -> Vec<String> {
    types.iter().map(|t| (*t).to_owned()).collect()
}

fn holds_all(_: CopyId) -> bool {
    true
}

#[test]
fn a_copy_holds_once_its_own_types_come_back() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a", "b"]));
    assert_eq!(b.state(CopyId(1)), Some(CopyState::Pending));
    let changes = b.selection(C, Some(m(&["a", "b"])), &holds_all);
    assert_eq!(
        changes,
        [Change::Holding {
            selection: C,
            copy: CopyId(1)
        }]
    );
    assert_eq!(b.state(CopyId(1)), Some(CopyState::Holding));
    assert_eq!(
        b.offered(C),
        Some(&Offered {
            mimes: m(&["a", "b"]),
            ours: Some(CopyId(1))
        })
    );
}

#[test]
fn another_apps_copy_is_offered_and_ours_is_lost_as_taken() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    b.selection(C, Some(m(&["a"])), &holds_all);
    // The platform cancels our source first, then announces the other app's offer.
    assert_eq!(
        b.cancelled(CopyId(1), false),
        [Change::Lost {
            selection: C,
            copy: CopyId(1),
            why: Lost::Taken
        }]
    );
    assert_eq!(b.offered(C).and_then(|o| o.ours), None);
    let changes = b.selection(C, Some(m(&["a"])), &|_| false);
    assert_eq!(
        changes,
        [Change::Offered {
            selection: C,
            mimes: m(&["a"])
        }]
    );
}

#[test]
fn an_offer_with_the_same_types_isnt_ours_once_the_platform_says_our_source_lost() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    let changes = b.selection(C, Some(m(&["a"])), &|_| false);
    assert_eq!(
        changes,
        [Change::Offered {
            selection: C,
            mimes: m(&["a"])
        }]
    );
    assert_eq!(b.state(CopyId(1)), Some(CopyState::Pending));
}

#[test]
fn the_echo_goes_to_the_newest_copy_offering_exactly_those_types() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    b.copied(C, CopyId(2), m(&["a", "b"]));
    // The first copy's echo arrives before the platform cancels it for the second.
    assert_eq!(
        b.selection(C, Some(m(&["a"])), &holds_all),
        [Change::Holding {
            selection: C,
            copy: CopyId(1)
        }]
    );
    assert_eq!(
        b.cancelled(CopyId(1), false),
        [Change::Lost {
            selection: C,
            copy: CopyId(1),
            why: Lost::Replaced
        }]
    );
    assert_eq!(
        b.selection(C, Some(m(&["a", "b"])), &holds_all),
        [Change::Holding {
            selection: C,
            copy: CopyId(2)
        }]
    );
}

#[test]
fn a_write_the_platform_judged_dropped_is_refused_and_any_other_pending_cancel_taken() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    assert_eq!(
        b.cancelled(CopyId(1), true),
        [Change::Lost {
            selection: C,
            copy: CopyId(1),
            why: Lost::Refused
        }]
    );
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    assert_eq!(
        b.cancelled(CopyId(1), false),
        [Change::Lost {
            selection: C,
            copy: CopyId(1),
            why: Lost::Taken
        }],
        "a pending copy cancelled without being judged dropped was taken"
    );
}

#[test]
fn a_clear_is_reported_as_cleared_and_a_late_second_cancel_says_nothing() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    b.selection(C, Some(m(&["a"])), &holds_all);
    assert!(b.clearing(CopyId(1), true));
    assert_eq!(
        b.cancelled(CopyId(1), false),
        [Change::Lost {
            selection: C,
            copy: CopyId(1),
            why: Lost::Cleared
        }]
    );
    assert!(b.cancelled(CopyId(1), false).is_empty());
    assert!(!b.clearing(CopyId(1), true), "a lost copy can't be cleared");
}

#[test]
fn an_emptied_selection_is_said_once() {
    let mut b = Book::default();
    b.selection(C, Some(m(&["a"])), &|_| false);
    assert_eq!(
        b.selection(C, None, &holds_all),
        [Change::Emptied { selection: C }]
    );
    assert!(b.selection(C, None, &holds_all).is_empty());
}

#[test]
fn selections_are_kept_apart() {
    let mut b = Book::default();
    b.copied(Selection::Primary, CopyId(1), m(&["a"]));
    assert_eq!(
        b.selection(C, Some(m(&["a"])), &holds_all),
        [Change::Offered {
            selection: C,
            mimes: m(&["a"])
        }],
        "a primary copy isn't the clipboard's"
    );
    assert_eq!(
        b.selection(Selection::Primary, Some(m(&["a"])), &holds_all),
        [Change::Holding {
            selection: Selection::Primary,
            copy: CopyId(1)
        }]
    );
}

#[test]
fn a_failed_connection_loses_every_copy_and_empties_every_selection() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    b.copied(Selection::Primary, CopyId(2), m(&["b"]));
    b.selection(C, Some(m(&["a"])), &holds_all);
    let changes = b.failed();
    assert_eq!(
        changes,
        [
            Change::Lost {
                selection: C,
                copy: CopyId(1),
                why: Lost::Failed
            },
            Change::Lost {
                selection: Selection::Primary,
                copy: CopyId(2),
                why: Lost::Failed
            },
            Change::Emptied { selection: C },
        ]
    );
    assert!(b.live(C).is_empty());
}

#[test]
fn finished_copies_are_forgotten_beyond_a_bound() {
    let mut b = Book::default();
    for i in 1..=300 {
        b.copied(C, CopyId(i), m(&["a"]));
        b.cancelled(CopyId(i), false);
    }
    assert_eq!(b.state(CopyId(1)), None);
    assert_eq!(b.state(CopyId(300)), Some(CopyState::Lost(Lost::Taken)));
}

#[test]
fn an_echo_listing_the_types_in_another_order_is_still_ours() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a", "b"]));
    assert_eq!(
        b.selection(C, Some(m(&["b", "a"])), &holds_all),
        [Change::Holding {
            selection: C,
            copy: CopyId(1)
        }]
    );
}

#[test]
fn a_pending_copy_is_served_until_another_apps_offer_is_heard_after_it() {
    let mut b = Book::default();
    b.copied(C, CopyId(1), m(&["a"]));
    assert_eq!(
        b.serving(C),
        Some(CopyId(1)),
        "a read right after the copy gets it"
    );
    // A write never confirmed (a data-device write the compositor dropped while another
    // client's event was taken for its echo, say) mustn't shadow what came after it.
    b.selection(C, Some(m(&["theirs"])), &|_| false);
    assert_eq!(b.serving(C), None);
    // A later copy of ours is served again, and once holding, stays served.
    b.copied(C, CopyId(2), m(&["b"]));
    assert_eq!(b.serving(C), Some(CopyId(2)));
    b.selection(C, Some(m(&["b"])), &holds_all);
    assert_eq!(b.serving(C), Some(CopyId(2)));
}
