//! C20d's page in process: what a choice sends and saves, the popover closing without a choice,
//! and C21's text menu on the search field. The scripts in `tests/scripts/c20d/` check the end
//! states AT hears.

mod c20d {
    use rune::runtime::ElementInfo;
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::controls::icon_picker::Choice;
    use strata_gallery::{Gallery, scenes};

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
        for _ in 0..30 {
            rt.frame(FRAME);
        }
        rt
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> ElementInfo {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no {key}"))
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let r = element(rt, key).rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        rt.frame(FRAME);
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(FRAME);
        rt.frame(FRAME);
    }

    #[test]
    fn each_choice_is_heard_once_and_saved_as_an_icon_name_or_an_emoji() {
        let mut rt = mount("c20d-icons-light");
        tap(&mut rt, "row-6");
        assert_eq!(rt.state.c20d.chosen, [Choice::icon("strata-terminal")]);
        assert_eq!(rt.state.c20d.current, Some(Choice::icon("strata-terminal")));

        tap(&mut rt, "c20d-emoji-button");
        assert!(rt.state.c20d.emoji.is_open());
        assert_eq!(focused(&rt).as_deref(), Some("c20d-emoji-field"));
        press(&mut rt, Key::Down);
        press(&mut rt, Key::Enter);
        let d = &rt.state.c20d;
        assert_eq!(d.chosen.len(), 2, "one intent per choice: {:?}", d.chosen);
        assert_eq!(d.chosen[1], Choice::emoji("😀"));
        // What Strata writes to `[custom_icons]`: `emoji:` and the emoji's own text.
        assert_eq!(d.chosen[1].text(), "😀");
        assert_eq!(d.recent, [Choice::emoji("😀")]);
        assert!(!d.emoji.is_open());
        assert_eq!(d.closed, 1);
        // The label draws it with U+FE0F (anomalyco/rune#102); the choice stays canonical.
        assert_eq!(d.emoji_label(), "Emoji  😀\u{fe0f}");
        assert_eq!(focused(&rt).as_deref(), Some("c20d-emoji-button"));
        // The inline icons no longer mark one: the saved icon is an emoji.
        assert_eq!(d.icons.current(), None);
    }

    #[test]
    fn escape_and_a_press_outside_close_the_popover_with_no_choice() {
        let mut rt = mount("c20d-icons-light");
        tap(&mut rt, "c20d-emoji-button");
        press(&mut rt, Key::Escape);
        assert!(!rt.state.c20d.emoji.is_open());
        assert_eq!(rt.state.c20d.closed, 1);

        tap(&mut rt, "c20d-emoji-button");
        assert!(rt.state.c20d.emoji.is_open());
        // A press outside the popover, on the dialog's Done: Rune's modal dismisses the popover
        // and keeps the press; the poll reports the close a frame later.
        tap(&mut rt, "c20d-done");
        rt.frame(FRAME);
        let d = &rt.state.c20d;
        assert!(!d.emoji.is_open());
        assert_eq!(d.closed, 2);
        assert!(d.chosen.is_empty());
        assert!(d.customize, "the press went to the popover, not to Done");
        assert_eq!(focused(&rt).as_deref(), Some("c20d-emoji-button"));
    }

    #[test]
    fn a_menu_press_on_the_search_field_opens_c21s_text_menu() {
        let mut rt = mount("c20d-catalog-light");
        let r = element(&rt, "c20d-catalog-field").rect;
        let (x, y) = (r[0] + 10.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.secondary_down(x, y);
        rt.frame(FRAME);
        rt.secondary_up(x, y);
        rt.frame(FRAME);
        rt.frame(FRAME);
        assert!(rt.state.c20d.fields.is_open());
    }
}
