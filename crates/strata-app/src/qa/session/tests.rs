// SPDX-License-Identifier: MIT

use std::cell::Cell;

use super::{presses_elsewhere, sway::Area};

#[test]
fn only_a_press_another_client_takes_starts_its_drag() {
    let window = Area {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 760.0,
    };
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
