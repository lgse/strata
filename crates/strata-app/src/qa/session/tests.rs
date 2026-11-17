// SPDX-License-Identifier: MIT

use std::cell::Cell;

use super::{presses_elsewhere, sway::Area};

/// Motions are paced, not waited for, only after a press another client takes: one outside
/// the window, the drag the runner starts from the foreign drag source. A press inside the
/// window is waited for, however slow it is to take it. While sway does not say where the
/// window is, a press the window never took is another client's, as before the window's box
/// was asked.
#[test]
fn only_a_press_another_client_takes_starts_its_drag() {
    let window = Area {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 760.0,
    };
    // What it is, the window's box, the press point, whether the window took it, whether the
    // press is another client's, and whether it was waited for.
    let cases = [
        ("inside", Some(window), (600, 400), true, false, true),
        (
            "inside, slow to take it",
            Some(window),
            (600, 400),
            false,
            false,
            true,
        ),
        ("at its corner", Some(window), (0, 0), true, false, true),
        (
            "just past its right edge",
            Some(window),
            (1200, 400),
            false,
            true,
            false,
        ),
        (
            "on the drag source",
            Some(window),
            (1320, 800),
            false,
            true,
            false,
        ),
        ("box unknown, taken", None, (600, 400), true, false, true),
        (
            "box unknown, not taken",
            None,
            (1320, 800),
            false,
            true,
            true,
        ),
    ];
    for (what, window, at, taken, elsewhere, waited) in cases {
        let asked = Cell::new(false);
        let took = || {
            asked.set(true);
            taken
        };
        assert_eq!(presses_elsewhere(window, at, took), elsewhere, "{what}");
        assert_eq!(asked.get(), waited, "{what}: waited for");
    }
}
