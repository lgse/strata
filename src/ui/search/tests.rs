// SPDX-License-Identifier: GPL-3.0-or-later

use super::intersects_viewport;

#[test]
fn only_rows_intersecting_the_viewport_are_visible() {
    assert!(intersects_viewport(100.0, 32.0, 90.0, 100.0));
    assert!(!intersects_viewport(58.0, 32.0, 90.0, 100.0));
    assert!(!intersects_viewport(190.0, 32.0, 90.0, 100.0));
}
