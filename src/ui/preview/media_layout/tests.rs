// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn media_fits_both_axes_without_cropping_or_exceeding_the_upscale_limit() {
    for (viewport, source, expected) in [
        ((1280, 800), (160, 90), (320, 180)),
        ((1280, 800), (3000, 1200), (1280, 512)),
        ((1280, 800), (600, 1800), (266, 800)),
        ((200, 100), (160, 90), (177, 100)),
        ((0, 0), (160, 90), (0, 0)),
    ] {
        assert_eq!(
            fitted_size(viewport.0, viewport.1, source.0, source.1),
            expected
        );
    }
}
