//! K1's page in process: what the goldens can't show. Each golden scene draws its theme in
//! that theme's own appearance, the arrow keys step through the themes and Omarchy variants,
//! and a theme switch animates its colours (WP done-when: "A theme switch in the gallery
//! animates"). K1 implements no input bullet, so it has no scripts (owner ruling Q8).

mod k1 {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_gallery::scenes::k1::{Shown, theme_of};
    use strata_gallery::{Gallery, scenes};
    use strata_kit::shape::LookParams;
    use strata_kit::theme::bundled;
    use strata_kit::theme::omarchy::Variant;

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

    /// Sends `key`, which the next frame delivers, and runs that frame.
    fn press(rt: &mut Runtime<Gallery>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(FRAME);
    }

    /// The glow's colour as the dialog card draws it this frame.
    fn glow(rt: &Runtime<Gallery>) -> String {
        let pose = rt.presented_pose("k1-glow").expect("the dialog card");
        pose.shadows().first().expect("its glow").color.hex()
    }

    fn glow_of(shown: Shown) -> String {
        let look = theme_of(shown).look().with_params(&LookParams::default());
        look.glow.color.alpha(look.glow.opacity).hex()
    }

    #[test]
    fn each_golden_scene_shows_its_theme_in_that_themes_appearance() {
        let page = scenes::all().find(|p| p.id == "k1").expect("the k1 page");
        for &(name, scene) in page.scenes {
            let mut g = Gallery::default();
            scene(&mut g);
            let dark = name.ends_with("-dark");
            assert_eq!(g.look.kit.dark, dark, "{name}");
            assert_eq!(g.dark, dark, "{name}");
            let theme = theme_of(g.theme.expect("a theme"));
            assert_eq!(theme.dark_background(), dark, "{name}");
        }
    }

    #[test]
    fn the_arrow_keys_step_through_the_bundled_themes_and_wrap() {
        let mut rt = mount("k1-curated-dark");
        let themes = bundled();
        let at = |rt: &Runtime<Gallery>| match rt.state.theme.expect("a theme") {
            Shown::Bundled(id) => themes.iter().position(|t| t.id == id).expect("bundled"),
            Shown::Omarchy { .. } => panic!("a bundled theme"),
        };
        let start = at(&rt);
        press(&mut rt, Key::Down);
        run(&mut rt, 0.1);
        assert_eq!(at(&rt), (start + 1) % themes.len());
        for _ in 0..=start + 1 {
            press(&mut rt, Key::Up);
        }
        run(&mut rt, 0.1);
        assert_eq!(
            at(&rt),
            themes.len() - 1,
            "Up from the first theme wraps to the last"
        );
        assert_eq!(rt.state.page, Some("k1"));
    }

    #[test]
    fn on_an_omarchy_scene_the_arrow_keys_step_through_the_variants() {
        let mut rt = mount("k1-omarchy-lighter-dark");
        let variant = |rt: &Runtime<Gallery>| match rt.state.theme.expect("a theme") {
            Shown::Omarchy { variant, .. } => variant,
            Shown::Bundled(_) => panic!("an Omarchy theme"),
        };
        press(&mut rt, Key::Down);
        assert_eq!(variant(&rt), Variant::Darker);
        press(&mut rt, Key::Down);
        assert_eq!(variant(&rt), Variant::Contrast);
        press(&mut rt, Key::Down);
        assert_eq!(variant(&rt), Variant::Lighter);
    }

    fn click(rt: &mut Runtime<Gallery>, key: &str) {
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no {key}"))
            .rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
    }

    fn bg(look: &strata_kit::look::Look) -> String {
        look.kit.c.bg_base.hex()
    }

    /// A theme picked from the list is the gallery's: the index and other pages wear it, K2
    /// builds on it, and Neutral puts the fallback back.
    #[test]
    fn a_theme_picked_from_the_list_applies_to_the_whole_gallery() {
        // Opened from the index, as the app starts.
        let mut rt = strata_gallery::runtime(Gallery::default(), Env::new(false, 1.0));
        rt.frame(0.0);
        rt.update(|g, _| {
            g.page = Some("k1");
            g.k1.query = rune::TextEdit::new("dracula");
        });
        run(&mut rt, 0.2);
        click(&mut rt, "k1-pick-dracula");
        let dracula = Shown::Bundled("dracula");
        assert_eq!(rt.state.theme, Some(dracula));
        let want = theme_of(dracula).look().with_params(&LookParams::default());
        assert_eq!(bg(&rt.state.look), bg(&want));
        assert!(rt.state.dark, "Dracula is dark");

        press(&mut rt, Key::Escape);
        assert_eq!(rt.state.page, None);
        assert_eq!(bg(&rt.state.look), bg(&want), "the index wears it");

        // K2 rebuilds the look from the theme, and leaving it puts the theme's look back.
        rt.update(|g, _| g.page = Some("k2"));
        rt.frame(FRAME);
        rt.key(KeyEvent {
            key: Key::Command('='),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        });
        rt.frame(FRAME);
        assert_ne!(
            rt.state.look.kit.ty.ui, want.kit.ty.ui,
            "K2 stepped the text size"
        );
        assert_eq!(bg(&rt.state.look), bg(&want), "on Dracula's colours");
        press(&mut rt, Key::Escape);
        assert_eq!(rt.state.look.kit.ty.ui, want.kit.ty.ui);
        assert_eq!(bg(&rt.state.look), bg(&want));

        rt.update(|g, _| {
            g.page = Some("k1");
            g.k1.query = rune::TextEdit::new("neutral");
        });
        run(&mut rt, 0.2);
        click(&mut rt, "k1-pick-neutral");
        assert_eq!(rt.state.theme, None);
        let fallback = strata_kit::look::Look::fallback(true);
        assert_eq!(bg(&rt.state.look), bg(&fallback), "Neutral keeps dark");
    }

    /// The WP's done-when: "A theme switch in the gallery animates." Mid-switch the glow is
    /// neither theme's; once the motion settles it is the new theme's.
    #[test]
    fn a_theme_switch_animates_its_colours() {
        let mut rt = mount("k1-curated-dark");
        run(&mut rt, 1.0);
        let before = rt.state.theme.expect("a theme");
        assert_eq!(glow(&rt), glow_of(before));

        press(&mut rt, Key::Down);
        let after = rt.state.theme.expect("a theme");
        assert_ne!(
            glow_of(before),
            glow_of(after),
            "the next theme has another accent"
        );
        run(&mut rt, FRAME);
        let mid = glow(&rt);
        assert_ne!(mid, glow_of(before), "the switch has started");
        assert_ne!(mid, glow_of(after), "the switch hasn't jumped to its end");

        run(&mut rt, 2.0);
        assert_eq!(glow(&rt), glow_of(after));
    }
}
