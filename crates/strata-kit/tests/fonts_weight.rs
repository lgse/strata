// SPDX-License-Identifier: MIT

mod fonts {
    use rune::TextStyle;
    use rune::text::TextSystem;
    use strata_kit::fonts::{FAMILIES, FAMILY, FONTS};

    #[test]
    fn the_wght_axis_follows_the_weight() {
        let _families = rune::text::with_families(FAMILY, FAMILY);
        let _text = TextSystem::new(FONTS, FAMILIES);
        // JetBrains Mono's 600-unit advance: this path measures the bundled face.
        let advance = rune::text::advance("iiiiiiiiii", TextStyle::new(100.0, 400.0));
        assert!(
            (advance - 600.0).abs() < 0.01,
            "{advance} pt, not JetBrains Mono's"
        );
        let ink = |weight| {
            rune::text::ink("l", TextStyle::new(100.0, weight))
                .expect("ink")
                .bounds[2]
        };
        let (regular, medium, semibold) = (ink(400.0), ink(500.0), ink(600.0));
        assert!(
            regular < medium && medium < semibold,
            "{regular} {medium} {semibold}"
        );
    }
}
