// SPDX-License-Identifier: MIT

use super::*;

fn pending() -> PendingRename {
    PendingRename {
        old_location: Location::local("/home/test/old"),
        new_location: None,
        old_name: "old".into(),
        new_name: "new".into(),
        generation: 1,
        monitor_has_new_location: false,
        reveal_generation: 0,
        scroll_value: None,
        source_position: None,
        state: PendingRenameState::Queued,
    }
}

#[test]
fn queued_rename_rejects_completion_before_dispatch() {
    let mut pending = pending();
    let operation = OperationRequestId(7);

    assert!(!pending.finish_dispatch(operation));
    assert!(!pending.complete(operation, None));
    assert!(matches!(pending.state, PendingRenameState::Queued));
}

#[test]
fn dispatch_can_begin_only_once() {
    let mut pending = pending();

    assert!(pending.begin_dispatch());
    assert!(!pending.begin_dispatch());
    assert!(matches!(pending.state, PendingRenameState::Dispatching));
}

#[test]
fn running_rename_accepts_only_its_operation() {
    let mut pending = pending();
    let operation = OperationRequestId(11);
    let stale = OperationRequestId(10);

    assert!(pending.begin_dispatch());
    assert!(pending.owns_operation(operation, Some(operation)));
    assert!(!pending.owns_operation(operation, None));
    assert!(pending.finish_dispatch(operation));
    assert!(pending.owns_operation(operation, None));
    assert!(!pending.owns_operation(stale, Some(operation)));
    assert!(!pending.complete(stale, Some(operation)));
    assert!(pending.complete(operation, Some(operation)));
    assert!(matches!(
        pending.state,
        PendingRenameState::AwaitingRefresh { .. }
    ));
}

#[test]
fn changed_source_position_invalidates_scroll_while_listing_loads() {
    let scroll_value = Cell::new(Some(42.0));

    assert!(prepare_rename_reveal(Some(3), 7, true, &scroll_value));
    assert_eq!(scroll_value.get(), None);
}

#[test]
fn synchronous_completion_preserves_refresh_state() {
    let mut pending = pending();
    let operation = OperationRequestId(12);

    assert!(pending.begin_dispatch());
    assert!(pending.complete(operation, Some(operation)));
    assert!(!pending.finish_dispatch(operation));
    assert!(matches!(
        pending.state,
        PendingRenameState::AwaitingRefresh { .. }
    ));
}
