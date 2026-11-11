//! The bundled face's `wght` axis, measured through `rune::text`'s shared measure path.
//!
//! This test has a process of its own: the measure path's fonts can't be pinned while other
//! tests in the process build text systems. Rune's `with_text` reads the fonts generation after
//! building a thread's measurer, without a lock, so a parallel `TextSystem::new` can tag a
//! measurer holding Inter with JetBrains Mono's generation, and the measurer then keeps Inter
//! under JetBrains Mono's name (Rune draft R-K6-1, `docs/testing.md`). In the lib tests this
//! failed about one run in six.

mod fonts {
    use rune::TextStyle;
    use rune::text::TextSystem;
    use strata_kit::fonts::{FAMILIES, FAMILY, FONTS};

    /// The face is variable: a heavier weight draws a wider stem, so medium and semibold text
    /// doesn't draw as regular.
    #[test]
    fn the_wght_axis_follows_the_weight() {
        // The guard makes this thread's measure state its own; the text system built under it
        // lands the bundled fonts there.
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
