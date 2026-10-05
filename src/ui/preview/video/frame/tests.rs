// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn placeholders_fit_the_aspect_inside_the_frame_area() {
    let wide = fitted(700.0, 600.0, 16.0 / 9.0);
    assert_eq!(
        (wide.x(), wide.y(), wide.width(), wide.height()),
        (0.0, 103.0, 700.0, 393.0)
    );
    let tall = fitted(700.0, 600.0, 9.0 / 16.0);
    assert_eq!(
        (tall.x(), tall.y(), tall.width(), tall.height()),
        (181.0, 0.0, 337.0, 600.0)
    );
    let shaped = fitted(640.0, 359.0, 16.0 / 9.0);
    assert_eq!(
        (shaped.x(), shaped.y(), shaped.width(), shaped.height()),
        (0.0, 0.0, 640.0, 359.0),
        "a rounding-only difference fills the allocation"
    );
}
