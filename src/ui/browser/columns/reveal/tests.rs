// SPDX-License-Identifier: MIT

use super::*;

fn span(depth: usize, trailing: usize) -> ColumnSpan {
    ColumnSpan {
        left: 300.0 * depth as f64,
        right: 300.0 * (depth + 1) as f64,
        trailing: 300.0 * trailing as f64,
    }
}

#[test]
fn reveal_target_moves_only_clipped_columns_and_aligns_the_strip_with_the_right_pane() {
    let upper = 1500.0;
    let cases = [
        (
            "a fully visible column stays put",
            span(1, 0),
            200.0,
            400.0,
            200.0,
        ),
        (
            "clipped on the right meets the right pane",
            span(3, 0),
            100.0,
            400.0,
            800.0,
        ),
        (
            "clipped on the left also meets the right pane",
            span(1, 0),
            700.0,
            400.0,
            200.0,
        ),
        (
            "a trailing child that fits stays beside it",
            span(2, 1),
            0.0,
            700.0,
            500.0,
        ),
        (
            "overflowing trailing columns leave the focused column flush left",
            span(1, 3),
            700.0,
            1000.0,
            300.0,
        ),
        (
            "the lent viewport lands on the same offset as the file state",
            span(2, 1),
            0.0,
            600.0,
            600.0,
        ),
        (
            "targets stay within the scroll range",
            span(4, 0),
            0.0,
            1600.0,
            0.0,
        ),
    ];
    for (name, span, current, page, expected) in cases {
        assert_eq!(
            span.reveal_target(current, page, 0.0, upper),
            expected,
            "{name}"
        );
    }
}
