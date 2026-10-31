// SPDX-License-Identifier: MIT

use super::stepped;

#[test]
fn steps_move_along_the_size_ladder_and_stop_at_its_ends() {
    for (size, steps, expected) in [
        (64, 1, 72),
        (64, -1, 56),
        (64, 3, 96),
        (64, -3, 44),
        (32, -1, 32),
        (256, 1, 256),
        (48, 40, 256),
        (48, -40, 32),
        (64, 0, 64),
    ] {
        assert_eq!(stepped(size, steps), expected, "{size} by {steps}");
    }
}

#[test]
fn a_size_between_steps_moves_to_the_nearer_step_in_that_direction() {
    for (size, steps, expected) in [(100, 1, 112), (100, -1, 96), (100, 2, 128), (100, -2, 80)] {
        assert_eq!(stepped(size, steps), expected, "{size} by {steps}");
    }
}
