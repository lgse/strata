//! W2's page: the gallery's look, which pages draw with and look pages change.

mod w2 {
    use rune::Env;
    use strata_gallery::{Gallery, scenes};
    use strata_kit::tokens::{self, Value};

    fn scene(name: &str) -> fn(&mut Gallery) {
        scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"))
    }

    fn texts(g: Gallery) -> String {
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        rt.accessibility_json().to_string()
    }

    #[test]
    fn opening_a_page_rebuilds_the_look_for_its_appearance() {
        let mut g = Gallery::default();
        tokens::set(&mut g.look, "row.list", Value::Length(99.0)).expect("set");
        g.open("w2", true);
        assert!(g.look().kit.dark);
        assert_eq!(tokens::get(g.look(), "row.list"), Ok(Value::Length(26.0)));
        g.open("w2", false);
        assert!(!g.look().kit.dark);
    }

    #[test]
    fn the_index_scenes_draw_on_the_look_for_their_appearance() {
        for (name, dark) in [("gallery-index-light", false), ("gallery-index-dark", true)] {
            let mut g = Gallery::default();
            scene(name)(&mut g);
            assert_eq!(g.kit().dark, dark, "{name}");
        }
    }

    #[test]
    fn a_page_draws_what_a_look_page_set() {
        let tokens_scene = scene("w2-tokens-light");
        let mut fallback = Gallery::default();
        tokens_scene(&mut fallback);
        assert!(texts(fallback).contains("row.list 26"));

        let mut changed = Gallery::default();
        tokens_scene(&mut changed);
        tokens::set(&mut changed.look, "row.list", Value::Length(30.0)).expect("set");
        let tree = texts(changed);
        assert!(
            tree.contains("row.list 30"),
            "the sheet shows the changed token"
        );
        assert!(!tree.contains("row.list 26"));
    }
}
