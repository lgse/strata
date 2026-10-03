// SPDX-License-Identifier: MIT

use super::*;

fn span(depth: usize) -> ColumnSpan {
    ColumnSpan {
        left: 300.0 * depth as f64,
        right: 300.0 * (depth + 1) as f64,
    }
}

#[test]
fn reveal_target_moves_only_clipped_columns_and_keeps_one_peek_sliver() {
    let upper = 1500.0;
    let cases = [
        (
            "fully visible column stays put",
            span(1),
            200.0,
            400.0,
            200.0,
        ),
        (
            "clipped on the right meets the right pane",
            span(3),
            100.0,
            400.0,
            800.0,
        ),
        (
            "the first column has no neighbour to peek",
            span(0),
            200.0,
            400.0,
            0.0,
        ),
        (
            "clipped on the left keeps a sliver of its parent",
            span(1),
            700.0,
            400.0,
            252.0,
        ),
        (
            "a wider viewport reveals the same way",
            span(3),
            100.0,
            700.0,
            500.0,
        ),
        (
            "targets stay within the scroll range",
            span(4),
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
