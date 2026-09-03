// SPDX-License-Identifier: GPL-3.0-or-later

use super::{code_panel_bounds, selection_span_bounds};

#[test]
fn code_panel_spans_from_the_first_through_the_last_line() {
    assert_eq!(
        code_panel_bounds(26, 40, 100, 18, 300.0, true, true),
        Some([16.0, 46.0, 284.0, 66.0])
    );
}

#[test]
fn selection_span_covers_only_the_visible_text_width() {
    assert_eq!(
        selection_span_bounds(26, 130, 54, 18),
        Some([26.0, 54.0, 104.0, 18.0])
    );
    assert_eq!(
        selection_span_bounds(130, 26, 54, 18),
        Some([26.0, 54.0, 104.0, 18.0])
    );
    assert_eq!(selection_span_bounds(26, 26, 54, 18), None);
}
