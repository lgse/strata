// SPDX-License-Identifier: MIT

use super::scale_radii;

#[test]
fn pixel_radii_scale_while_circles_and_other_lengths_stay() {
    let cases = [
        (
            "a { border-radius: 5px; }",
            0.0,
            "a { border-radius: 0px; }",
        ),
        (
            "a { border-radius: 5px; }",
            2.0,
            "a { border-radius: 10px; }",
        ),
        (
            "a { border-radius: 5px; }",
            0.5,
            "a { border-radius: 2.5px; }",
        ),
        (
            "a { border-radius: 9px; }",
            0.4,
            "a { border-radius: 3.6px; }",
        ),
        ("a { border-radius: 0; }", 2.0, "a { border-radius: 0; }"),
        (
            "a { border-radius: 50%; }",
            0.0,
            "a { border-radius: 50%; }",
        ),
        (
            "a { border-radius: 8px 8px 0 0; }",
            1.5,
            "a { border-radius: 12px 12px 0 0; }",
        ),
        (
            "a { border-top-left-radius: 4px }",
            0.0,
            "a { border-top-left-radius: 0px }",
        ),
        (
            "a { padding: 5px; border-radius: 999px; margin: 5px; }",
            0.0,
            "a { padding: 5px; border-radius: 0px; margin: 5px; }",
        ),
        (
            "a { border-radius: 6px; }\nb { border-radius: 4px; box-shadow: 0 0 4px red; }",
            2.0,
            "a { border-radius: 12px; }\nb { border-radius: 8px; box-shadow: 0 0 4px red; }",
        ),
    ];
    for (css, scale, expected) in cases {
        assert_eq!(scale_radii(css, scale), expected, "{css} at {scale}");
    }
}
