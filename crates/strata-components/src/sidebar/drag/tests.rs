// SPDX-License-Identifier: MIT

use super::{Landing, Rows, landing};

/// Rows 24 points tall with 2 between, so a row's middle is 26 from the next one's.
const ROWS: Rows = Rows {
    height: 24.0,
    pitch: 26.0,
};

const fn row(index: usize, after: bool) -> Option<Landing> {
    Some(Landing::Row { index, after })
}

/// Where a dragged row lands by Strata's rule, at the half-row boundaries, in the gaps, at
/// either end and past them. Row 2 of 3 dragged up: row 1 spans -38 to -14 from row 2's
/// middle, its middle at -26, and the gap above row 2 is -14 to -12.
#[test]
fn a_drop_lands_on_the_row_under_the_pointer_by_its_half_or_in_a_gap_or_nowhere() {
    // (from, count, pointer past the row's middle, where it lands, where it goes, what is under
    // the pointer)
    let cases = [
        (
            2,
            3,
            -30.0,
            row(1, false),
            1,
            "the upper half of the row above",
        ),
        (
            2,
            3,
            -20.0,
            row(1, true),
            2,
            "the lower half of the row above",
        ),
        (
            2,
            3,
            -26.0,
            row(1, true),
            2,
            "the exact middle of the row above: after it",
        ),
        (
            2,
            3,
            -13.0,
            Some(Landing::Gap(2)),
            2,
            "the gap above its own row",
        ),
        (
            2,
            3,
            -39.0,
            Some(Landing::Gap(1)),
            1,
            "the gap between rows 0 and 1",
        ),
        (
            0,
            3,
            39.0,
            Some(Landing::Gap(2)),
            1,
            "the gap between rows 1 and 2",
        ),
        (
            0,
            3,
            30.0,
            row(1, true),
            1,
            "the lower half of the row below",
        ),
        (
            0,
            3,
            20.0,
            row(1, false),
            0,
            "the upper half of the row below",
        ),
        (
            0,
            3,
            -13.0,
            Some(Landing::Gap(0)),
            0,
            "the gap above the first row",
        ),
        (
            0,
            3,
            65.0,
            Some(Landing::Gap(3)),
            2,
            "the gap below the last row",
        ),
        (1, 3, 0.0, row(1, true), 1, "its own middle"),
    ];
    for (from, count, along, expected, place, under) in cases {
        let landed = landing(from, count, along, ROWS);
        assert_eq!(landed, expected, "{under}");
        let placed = landed.map(|landed| landed.place(from, count));
        assert_eq!(placed, Some(place), "{under}");
    }
}

/// Past the gap at either end of a block, which is where the other block or the sidebar's edge
/// is, Strata's sidebar takes no drop, where rune-kit's `Reorder` clamped to the end.
#[test]
fn a_drop_past_either_end_of_its_block_lands_nowhere() {
    let cases = [
        (2, 3, -70.0, "above the first row's gap"),
        (0, 3, 75.0, "below the last row's gap"),
        (0, 1, 40.0, "past a lone row"),
    ];
    for (from, count, along, under) in cases {
        assert_eq!(landing(from, count, along, ROWS), None, "{under}");
    }
}
