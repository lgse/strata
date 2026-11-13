// SPDX-License-Identifier: MIT

use super::landing;

/// Rows 24 points tall with 2 between, so a row's middle is 26 from the next one's.
const PITCH: f32 = 26.0;

/// Where a dragged row lands by Strata's rule, around the half-row boundaries and in the gaps.
/// Row 2 of 3 dragged up: row 1 spans -38 to -14 from row 2's middle, its middle at -26, and
/// the gap above row 2 is -14 to -12.
#[test]
fn a_row_lands_before_or_after_the_row_under_the_pointer_by_its_half_or_beside_a_gap() {
    // (from, count, pointer past the row's middle, where it lands, what is under the pointer)
    let cases = [
        (2, 3, -30.0, 1, "the upper half of the row above: before it"),
        (
            2,
            3,
            -20.0,
            2,
            "the lower half of the row above: after it, its own place",
        ),
        (2, 3, -13.0, 2, "the gap above it: its own place"),
        (2, 3, -39.0, 1, "the gap between rows 0 and 1: after row 0"),
        (0, 3, 39.0, 1, "the gap between rows 1 and 2: after row 1"),
        (0, 3, 30.0, 1, "the lower half of the row below: after it"),
        (
            0,
            3,
            20.0,
            0,
            "the upper half of the row below: before it, its own place",
        ),
        (0, 3, 90.0, 2, "past the block's end: its end"),
        (2, 3, -90.0, 0, "past the block's start: its start"),
        (1, 3, 0.0, 1, "its own middle: its own place"),
    ];
    for (from, count, along, expected, under) in cases {
        assert_eq!(landing(from, count, along, PITCH), expected, "{under}");
    }
}
