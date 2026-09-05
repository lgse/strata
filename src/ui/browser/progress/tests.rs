// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn transfer_progress_reports_a_byte_fraction_when_the_total_is_known() {
    assert_eq!(
        transfer_progress_status(0, 2, 750, Some(1_000)),
        ("75%".to_owned(), Some(0.75))
    );
    assert_eq!(
        transfer_progress_status(1, 2, 1_500, Some(1_000)),
        ("100%".to_owned(), Some(1.0))
    );
    assert_eq!(
        transfer_progress_status(0, 2, 1, Some(1_000)),
        ("1%".to_owned(), Some(0.001))
    );
}

#[test]
fn zero_byte_transfer_progress_tracks_completed_items() {
    assert_eq!(
        transfer_progress_status(0, 2, 0, Some(0)),
        ("0%".to_owned(), Some(0.0))
    );
    assert_eq!(
        transfer_progress_status(1, 2, 0, Some(0)),
        ("50%".to_owned(), Some(0.5))
    );
    assert_eq!(
        transfer_progress_status(2, 2, 0, Some(0)),
        ("100%".to_owned(), Some(1.0))
    );
    assert_eq!(
        transfer_progress_status(0, 0, 0, Some(0)),
        ("Preparing…".to_owned(), None)
    );
}

#[test]
fn transfer_progress_is_indeterminate_when_the_total_is_unknown() {
    assert_eq!(
        transfer_progress_status(0, 2, 0, None),
        ("Preparing…".to_owned(), None)
    );
    assert_eq!(
        transfer_progress_status(0, 2, 1_200, None),
        ("1.2 kB copied".to_owned(), None)
    );
}

#[test]
fn small_operations_delay_progress_while_large_or_unbounded_operations_show_it_immediately() {
    assert!(!should_show_progress_immediately(1));
    assert!(!should_show_progress_immediately(
        IMMEDIATE_PROGRESS_ITEM_COUNT - 1
    ));
    assert!(should_show_progress_immediately(
        IMMEDIATE_PROGRESS_ITEM_COUNT
    ));
    assert!(should_show_progress_immediately(0));
}
