// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn the_glow_needs_the_preference_animations_and_a_gpu_renderer() {
    for (element_glow, animations, software, expected) in [
        (true, true, false, true),
        (false, true, false, false),
        (true, false, false, false),
        (true, true, true, false),
        (false, false, true, false),
    ] {
        assert_eq!(
            allowed(element_glow, animations, software),
            expected,
            "glow={element_glow} animations={animations} software={software}"
        );
    }
}
