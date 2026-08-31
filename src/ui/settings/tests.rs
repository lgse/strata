// SPDX-License-Identifier: GPL-3.0-or-later

use super::{DIALOG_HEIGHT, DIALOG_MARGIN, DIALOG_WIDTH, responsive_dialog_size};

#[test]
fn settings_dialog_keeps_its_preferred_size_when_space_allows() {
    assert_eq!(
        responsive_dialog_size(
            DIALOG_WIDTH + DIALOG_MARGIN * 2,
            DIALOG_HEIGHT + DIALOG_MARGIN * 2,
        ),
        (DIALOG_WIDTH, DIALOG_HEIGHT)
    );
}

#[test]
fn settings_dialog_shrinks_to_leave_a_margin_in_small_windows() {
    assert_eq!(responsive_dialog_size(640, 480), (592, 432));
}

#[test]
fn settings_dialog_size_stays_valid_at_tiny_allocations() {
    assert_eq!(responsive_dialog_size(20, 20), (1, 1));
}
