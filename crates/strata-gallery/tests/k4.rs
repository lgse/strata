//! K4's page in process: what the goldens can't show. Each golden scene draws its theme with
//! the page's file applied last, and turning the overrides off and on goes through
//! `theme::switch`, as a reload of the file does, so the colours animate. K4 implements no
//! input bullet, so it has no scripts (as K1's ruling Q8).

mod k4 {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_gallery::{Gallery, scenes};
    use strata_kit::overrides::Overrides;
    use strata_kit::slots;
    use strata_kit::theme::Rgba8;

    const FRAME: f64 = 1.0 / 60.0;

    fn mount(name: &str) -> Runtime<Gallery> {
        let scene = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        scene(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        rt
    }

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn press_o(rt: &mut Runtime<Gallery>) {
        rt.key(KeyEvent {
            key: Key::Text("o".into()),
            mods: Mods::default(),
        });
        rt.frame(FRAME);
    }

    fn hex(s: &str) -> rune::Color {
        Rgba8::parse(s).expect("a colour").color()
    }

    /// The focused file row's glow as drawn this frame: the focus colour.
    fn glow(rt: &Runtime<Gallery>) -> String {
        let pose = rt.presented_pose("k4-focused").expect("the focused row");
        pose.shadows().first().expect("its glow").color.hex()
    }

    #[test]
    fn each_golden_scene_applies_its_file_last_in_its_themes_appearance() {
        let page = scenes::all().find(|p| p.id == "k4").expect("the k4 page");
        for &(name, scene) in page.scenes {
            let mut g = Gallery::default();
            scene(&mut g);
            let dark = name.ends_with("-dark");
            assert_eq!(g.look.kit.dark, dark, "{name}");
            assert!(g.k4.on && !g.k4.overrides.is_empty(), "{name}");
            if name.starts_with("k4-overrides") {
                assert_eq!(slots::sidebar::row_height(&g.look), 34.0, "{name}");
                let highlight = if dark { "#5a3a20" } else { "#ffd8b8" };
                assert_eq!(g.look.fill.highlight, hex(highlight), "{name}");
                assert_eq!(g.k4.overrides.errors().len(), 4, "{name}");
                // Colours taken from a token read its override.
                let focus = hex("#ff9e64");
                assert_eq!(slots::sidebar::selected_icon(&g.look), focus, "{name}");
                assert_eq!(
                    slots::sidebar::selected_fill(&g.look),
                    g.look.kit.alpha(focus, 0.25),
                    "{name}"
                );
            } else {
                // What the stylesheet migrated to is what the page draws with.
                let m = g.k4.migration.as_ref().expect("a migration scene");
                assert_eq!(g.k4.overrides, Overrides::parse(&m.toml), "{name}");
                assert_eq!(
                    slots::sidebar::selected_fill(&g.look),
                    hex("#ff9e6440"),
                    "{name}"
                );
                assert_eq!(slots::sidebar::selected_icon(&g.look), hex("#ff9e64"));
            }
        }
    }

    /// A reload goes through the same path as a theme switch (WP scope): mid-way the focus
    /// glow is neither colour, and it settles on the theme's, then back on the override's.
    #[test]
    fn turning_the_overrides_off_and_on_animates_as_a_reload() {
        let mut rt = mount("k4-overrides-dark");
        run(&mut rt, 1.0);
        let theme = rt.state.k4.base.kit.c.border_focus;
        let opacity = rt.state.look.glow.opacity;
        let ours = hex("#ff9e64").alpha(opacity).hex();
        let theirs = theme.alpha(opacity).hex();
        assert_ne!(ours, theirs);
        assert_eq!(glow(&rt), ours);

        press_o(&mut rt);
        assert!(!rt.state.k4.on);
        assert_eq!(
            slots::sidebar::row_height(&rt.state.look),
            slots::sidebar::row_height(&rt.state.k4.base)
        );
        run(&mut rt, FRAME);
        let mid = glow(&rt);
        assert!(mid != ours && mid != theirs, "{mid}");
        run(&mut rt, 2.0);
        assert_eq!(glow(&rt), theirs);

        press_o(&mut rt);
        run(&mut rt, 2.0);
        assert!(rt.state.k4.on);
        assert_eq!(glow(&rt), ours);
        assert_eq!(slots::sidebar::row_height(&rt.state.look), 34.0);
    }
}
