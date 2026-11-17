// SPDX-License-Identifier: MIT

use super::{presses_elsewhere, sway::Area};

/// Motions are paced, not waited for, only after a press outside the window, which another
/// client takes: the drag the runner starts from the foreign drag source. A press inside the
/// window is waited for, however slow it is to take it, and so is one while the window's
/// place is unknown.
#[test]
fn only_a_press_outside_the_window_starts_another_clients_drag() {
    let window = Area {
        x: 0.0,
        y: 0.0,
        width: 1200.0,
        height: 760.0,
    };
    let cases = [
        ("inside", Some(window), (600, 400), false),
        ("at its corner", Some(window), (0, 0), false),
        ("just past its right edge", Some(window), (1200, 400), true),
        (
            "on the drag source past its corner",
            Some(window),
            (1320, 800),
            true,
        ),
        ("with the window's place unknown", None, (1320, 800), false),
    ];
    for (what, window, at, elsewhere) in cases {
        assert_eq!(presses_elsewhere(window, at), elsewhere, "{what}");
    }
}
