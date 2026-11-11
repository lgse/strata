//! K2's page in process: what the accessibility tree doesn't show. The interface font
//! reaches Rune's sans slot (`Runtime::font_families`), mono stays the app's (the bundled
//! JetBrains Mono, K6), and Default and Escape put the app's families back. The scripts in
//! `tests/scripts/k2/` check the rest.

mod k2 {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_gallery::scenes::k2::APP_FAMILIES;
    use strata_gallery::{Gallery, scenes};
    use strata_kit::look::Look;

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

    fn run(rt: &mut Runtime<Gallery>) {
        for _ in 0..6 {
            rt.frame(FRAME);
        }
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        run(rt);
    }

    fn families(rt: &Runtime<Gallery>) -> (String, String) {
        let (sans, mono) = rt.font_families();
        (sans.to_owned(), mono.to_owned())
    }

    fn app() -> (String, String) {
        (APP_FAMILIES.0.to_owned(), APP_FAMILIES.1.to_owned())
    }

    fn pick(rt: &mut Runtime<Gallery>, key: &str) {
        tap(rt, "k2-font-open");
        tap(rt, key);
    }

    /// settings/themes/interface-font: a change applies at once; previews keep the
    /// monospace font; an uninstalled family falls back to the app's own; Default restores
    /// the bundled font.
    #[test]
    fn a_chosen_family_leads_the_sans_slot_and_mono_stays_the_apps() {
        let mut rt = mount("k2-defaults-light");
        let bundled = ("JetBrains Mono".to_owned(), "JetBrains Mono".to_owned());
        assert_eq!(families(&rt), bundled);
        pick(&mut rt, "k2-font-1");
        assert_eq!(
            families(&rt),
            (format!("\"DejaVu Sans\", {}", APP_FAMILIES.0), app().1)
        );
        pick(&mut rt, "k2-font-5");
        assert_eq!(
            families(&rt).0,
            format!("'Other \"Quoted\" Sans', {}", APP_FAMILIES.0)
        );
        pick(&mut rt, "k2-font-default");
        assert_eq!(families(&rt), bundled);
    }

    #[test]
    fn escape_puts_the_apps_families_and_the_fallback_look_back_and_leaves_the_page() {
        let mut rt = mount("k2-round-light");
        pick(&mut rt, "k2-font-1");
        assert_ne!(families(&rt), app());
        let fallback = Look::fallback(false);
        assert_ne!(rt.state.look.roundness, fallback.roundness);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt);
        assert_eq!(families(&rt), app());
        assert_eq!(rt.state.page, None, "the gallery went back to the index");
        let look = &rt.state.look;
        assert_eq!(look.roundness, fallback.roundness);
        assert_eq!(look.kit.radius.md, fallback.kit.radius.md);
        assert_eq!((look.row, look.frame), (fallback.row, fallback.frame));
        assert_eq!(look.font, None);
    }

    #[test]
    fn escape_closes_an_open_picker_first() {
        let mut rt = mount("k2-defaults-light");
        tap(&mut rt, "k2-font-open");
        assert!(rt.find("k2-font-list").is_some());
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt);
        assert!(rt.find("k2-font-list").is_none());
        assert_eq!(rt.state.page, Some("k2"));
    }

    /// browser/view-modes/text-size: the zoom keys with Alt or Super held do nothing.
    #[test]
    fn zoom_keys_with_alt_or_super_held_leave_the_size_alone() {
        let mut rt = mount("k2-defaults-light");
        for mods in [
            Mods {
                cmd: true,
                ctrl: true,
                alt: true,
                ..Mods::default()
            },
            Mods {
                cmd: true,
                ctrl: true,
                logo: true,
                ..Mods::default()
            },
        ] {
            rt.key(KeyEvent {
                key: Key::Command('='),
                mods,
            });
            run(&mut rt);
            assert_eq!(rt.state.k2.params.text_size, 13, "{mods:?}");
        }
    }
}
