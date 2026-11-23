// SPDX-License-Identifier: MIT

mod k6 {
    use rune::{Env, TextStyle};
    use strata_gallery::Gallery;

    /// JetBrains Mono's advance: 600 units on 1000 per em.
    const ADVANCE_EM: f32 = 0.6;

    #[test]
    fn the_gallery_runtime_shapes_in_the_bundled_font() {
        let _families = rune::text::with_families("JetBrains Mono", "JetBrains Mono");
        let mut rt = strata_gallery::runtime(Gallery::default(), Env::new(false, 1.0));
        rt.frame(0.0);
        assert_eq!(rt.font_families(), ("JetBrains Mono", "JetBrains Mono"));
        let style = TextStyle::new(100.0, 400.0);
        for style in [style, style.mono()] {
            for run in ["iiiiiiiiii", "MMMMMMMMMM"] {
                let w = rune::text::advance(run, style);
                assert!(
                    (w - 10.0 * 100.0 * ADVANCE_EM).abs() < 0.01,
                    "{run}: {w} pt, not JetBrains Mono's"
                );
            }
        }
    }
}
