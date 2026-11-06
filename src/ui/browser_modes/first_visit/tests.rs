// SPDX-License-Identifier: MIT

use super::FirstVisit;

#[test]
fn only_an_untouched_visit_finishing_at_its_own_depth_may_refocus() {
    let mut slot = Some(FirstVisit::new(1));
    assert!(
        !FirstVisit::take_untouched(&mut slot, 0),
        "a parent column's load must not report the child's visit"
    );
    assert!(
        slot.is_some(),
        "a parent column's load must leave the visit"
    );
    assert!(FirstVisit::take_untouched(&mut slot, 1));
    assert!(slot.is_none(), "the pane's own load ends the visit");
    assert!(!FirstVisit::take_untouched(&mut slot, 1));

    let mut slot = Some(FirstVisit::new(0));
    // What the pane's input controller does when user input reaches the pane.
    slot.as_ref().expect("visit").untouched.interrupt();
    assert!(
        !FirstVisit::take_untouched(&mut slot, 0),
        "input during the visit must keep the load from refocusing"
    );
    assert!(
        slot.is_none(),
        "an interrupted visit still ends with its load"
    );
}
