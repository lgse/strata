// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn parked_pointer_cannot_override_keyboard_navigation() {
    let mut input = InputOwnership::default();
    input.pointer_motion((50.0, 100.0));
    assert_eq!(input.destination(Some(0), Some(2), Some(2), 3), Some(0));
    input.keyboard_navigation();
    assert_eq!(input.destination(Some(0), Some(2), Some(2), 3), Some(2));
    assert!(!input.pointer_motion((50.0, 100.0)));
    assert_eq!(input.destination(Some(1), Some(2), Some(2), 3), Some(2));
    assert!(input.pointer_motion((51.0, 100.0)));
    assert_eq!(input.destination(Some(1), Some(2), Some(2), 3), Some(1));
}

#[test]
fn pointer_click_reclaims_ownership_without_motion() {
    let mut input = InputOwnership::default();
    input.pointer_action();
    assert_eq!(input.destination(Some(0), Some(1), Some(1), 2), Some(0));
}

#[test]
fn paste_resolution_does_not_change_input_ownership() {
    let mut input = InputOwnership::default();
    for method in [NavigationInput::Pointer, NavigationInput::Keyboard] {
        input.last_navigation = method;
        for _ in 0..3 {
            assert_eq!(
                input.destination(Some(0), Some(1), Some(2), 3),
                Some(if method == NavigationInput::Pointer {
                    0
                } else {
                    1
                })
            );
            assert_eq!(input.last_navigation, method);
        }
    }
}

#[test]
fn keyboard_targets_focused_parent_not_deepest_open_column() {
    let input = InputOwnership::default();
    assert_eq!(input.destination(None, Some(0), Some(2), 3), Some(0));
    assert_eq!(input.destination(None, None, Some(1), 3), Some(1));
}

#[test]
fn pointer_outside_columns_and_stale_depths_fall_back_safely() {
    let mut input = InputOwnership::default();
    input.pointer_action();
    assert_eq!(input.destination(None, Some(0), Some(2), 3), Some(0));
    assert_eq!(input.destination(Some(3), Some(4), Some(1), 2), Some(1));
    assert_eq!(input.destination(Some(3), None, None, 2), Some(1));
    assert_eq!(input.destination(Some(0), Some(0), Some(0), 0), None);
}
