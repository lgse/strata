//! C15's page in process: what the accessibility tree doesn't show (motion, the backdrop,
//! keyboard focus rings), and focus after closing by every route. These tests, not the
//! scripts, check the motion, blur and ring bullets; the scripts in `tests/scripts/c15/` check
//! the rest. Each test names the `app/dialogs` bullets it covers.

mod c15 {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::dialog::BACKDROP_KEY;
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
        rt
    }

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    fn focus_visible(rt: &Runtime<Gallery>, key: &str) -> bool {
        element(rt, key).is_some_and(|e| e.focused && e.focus_visible)
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key, shift: bool) {
        rt.key(KeyEvent {
            key,
            mods: Mods {
                shift,
                ..Mods::default()
            },
        });
        run(rt, 0.1);
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
        tap_at(rt, x, y);
    }

    fn tap_at(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
    }

    /// Tabs from wherever focus is until `key` has it, from the keyboard.
    fn tab_to(rt: &mut Runtime<Gallery>, key: &str) {
        for _ in 0..40 {
            if focused(rt).as_deref() == Some(key) {
                return;
            }
            press(rt, Key::Tab, false);
        }
        panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
    }

    /// (opacity, scale, exiting) of the element keyed `key`.
    fn pose(rt: &Runtime<Gallery>, key: &str) -> Option<(f32, f32, bool)> {
        element(rt, key).map(|e| (e.opacity, e.scale, e.exiting))
    }

    /// app/dialogs: "Opening an action or search dialog fades in the backdrop and fades the
    /// dialog in from 96% scale over 200 ms; closing reverses it over 200 ms."
    #[test]
    fn a_dialog_scales_and_fades_in_and_out_in_about_200_ms() {
        let mut rt = mount("c15-openers-light");
        tap(&mut rt, "c15-open-danger");
        run(&mut rt, 0.05);
        let (opacity, scale, _) = pose(&rt, "c15-danger-layer").expect("opening");
        assert!(opacity > 0.0 && opacity < 1.0, "fading in: {opacity}");
        assert!((0.96..1.0).contains(&scale), "growing from 96%: {scale}");
        let (backdrop, _, _) = pose(&rt, BACKDROP_KEY).expect("the backdrop fades in");
        assert!(backdrop > 0.0 && backdrop < 1.0, "{backdrop}");
        run(&mut rt, 0.3);
        let (opacity, scale, _) = pose(&rt, "c15-danger-layer").expect("open");
        assert!(
            opacity > 0.97 && scale > 0.995,
            "at rest by 350 ms: {opacity} {scale}"
        );

        press(&mut rt, Key::Escape, false);
        let (opacity, scale, exiting) = pose(&rt, "c15-danger-layer").expect("closing");
        assert!(exiting && opacity < 1.0 && scale < 1.0, "{opacity} {scale}");
        run(&mut rt, 0.3);
        assert_eq!(pose(&rt, "c15-danger-layer"), None, "gone by 400 ms");
        assert_eq!(pose(&rt, BACKDROP_KEY), None);
    }

    /// app/dialogs: "Opening Settings fades its whole layer…", "The first Settings open of a
    /// process fades in…": the shell's fade-only panel, opened for the first time.
    #[test]
    fn a_fade_only_panel_fades_without_scaling_on_its_first_open() {
        let mut rt = mount("c15-openers-light");
        tap(&mut rt, "c15-open-panel");
        run(&mut rt, 0.05);
        let (opacity, scale, _) = pose(&rt, "c15-panel-layer").expect("opening");
        assert!(
            opacity > 0.0 && opacity < 1.0,
            "the first open fades: {opacity}"
        );
        assert_eq!(scale, 1.0, "no scale");
        press(&mut rt, Key::Escape, false);
        let (opacity, scale, exiting) = pose(&rt, "c15-panel-layer").expect("closing");
        assert!(exiting && opacity < 1.0);
        assert_eq!(scale, 1.0);
    }

    /// app/dialogs: "Closing Settings during its fade-in stops the fade-in and fades out from
    /// the current opacity."
    #[test]
    fn closing_mid_open_fades_out_from_where_it_was() {
        let mut rt = mount("c15-openers-light");
        tap(&mut rt, "c15-open-panel");
        run(&mut rt, 0.07);
        let (mid, _, _) = pose(&rt, "c15-panel-layer").expect("opening");
        assert!(mid > 0.1 && mid < 0.95, "{mid}");
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        let mut last = mid + 0.05;
        for _ in 0..6 {
            rt.frame(FRAME);
            let (opacity, _, exiting) = pose(&rt, "c15-panel-layer").expect("closing");
            assert!(exiting);
            assert!(opacity <= last, "falls from {mid}: {opacity} after {last}");
            last = opacity;
        }
    }

    /// app/dialogs: "With Reduce motion on, or GTK `gtk-enable-animations` off, Settings
    /// appears at full opacity at once and closes without a fade."
    #[test]
    fn with_animations_off_a_dialog_shows_and_goes_at_once() {
        let mut rt = mount("c15-reduced-light");
        for (opener, layer) in [
            ("c15-open-panel", "c15-panel-layer"),
            ("c15-open-danger", "c15-danger-layer"),
        ] {
            tap(&mut rt, opener);
            rt.frame(FRAME);
            assert_eq!(pose(&rt, layer), Some((1.0, 1.0, false)), "{layer}");
            press(&mut rt, Key::Escape, false);
            assert_eq!(pose(&rt, layer), None, "{layer}");
        }
    }

    /// app/dialogs: "While a dialog is open, the window content behind it is blurred; closing
    /// one of two stacked dialogs keeps the blur until the last one closes."
    #[test]
    fn the_backdrop_stays_until_the_last_stacked_dialog_closes() {
        let mut rt = mount("c15-openers-light");
        tap(&mut rt, "c15-open-outer");
        run(&mut rt, 0.3);
        tap(&mut rt, "c15-outer-permissions");
        run(&mut rt, 0.3);
        let backdrops = |rt: &Runtime<Gallery>| {
            rt.elements()
                .iter()
                .filter(|e| e.key.as_deref() == Some(BACKDROP_KEY) && !e.exiting)
                .count()
        };
        assert_eq!(backdrops(&rt), 1, "one blur under both");
        let blurred = element(&rt, BACKDROP_KEY).expect("backdrop");
        assert!(blurred.opacity > 0.99);
        press(&mut rt, Key::Escape, false);
        run(&mut rt, 0.3);
        assert!(element(&rt, "c15-inner-dialog").is_none());
        assert_eq!(backdrops(&rt), 1, "the blur stays under Properties");
        press(&mut rt, Key::Escape, false);
        run(&mut rt, 0.3);
        assert_eq!(backdrops(&rt), 0);
    }

    /// app/dialogs: "A keyboard-focused confirm button… shows a 3 px theme-accent halo",
    /// "That halo sits flush…", "Moving keyboard focus off the confirm button removes its
    /// halo." The ring's look is in the PNG of a keyboard-opened dialog, checked by hand.
    #[test]
    fn a_keyboard_focused_confirm_button_shows_its_ring_until_focus_moves() {
        let mut rt = mount("c15-openers-light");
        tab_to(&mut rt, "c15-open-danger");
        press(&mut rt, Key::Text(" ".into()), false);
        run(&mut rt, 0.3);
        assert!(focus_visible(&rt, "c15-danger-primary"));
        press(&mut rt, Key::Left, false);
        assert!(focus_visible(&rt, "c15-danger-secondary"));
        assert!(!element(&rt, "c15-danger-primary").is_some_and(|e| e.focus_visible));

        // Opened with the pointer, the primary has focus but shows no ring.
        let mut rt = mount("c15-openers-light");
        tap(&mut rt, "c15-open-danger");
        run(&mut rt, 0.3);
        assert_eq!(focused(&rt).as_deref(), Some("c15-danger-primary"));
        assert!(!focus_visible(&rt, "c15-danger-primary"));
    }

    /// The keyboard scenes' golden shows the ring: their poll puts focus in keyboard mode on
    /// the primary, which keeps focus.
    #[test]
    fn the_keyboard_scene_shows_the_primary_focused_from_the_keyboard() {
        let mut rt = mount("c15-danger-keyboard-light");
        run(&mut rt, 0.2);
        assert!(focus_visible(&rt, "c15-danger-primary"));
    }

    /// app/dialogs: "Closing the last open dialog by any route returns focus to the widget
    /// focused before the first one opened, if still on screen."
    #[test]
    fn every_route_that_closes_a_dialog_returns_focus_to_its_opener() {
        type Close = fn(&mut Runtime<Gallery>);
        let routes: [(&str, Close); 5] = [
            ("escape", |rt| press(rt, Key::Escape, false)),
            ("backdrop", |rt| tap_at(rt, 4.0, 630.0)),
            ("close button", |rt| tap(rt, "c15-danger-close")),
            ("cancel", |rt| tap(rt, "c15-danger-secondary")),
            ("confirm", |rt| press(rt, Key::Enter, false)),
        ];
        for (route, close) in routes {
            let mut rt = mount("c15-openers-light");
            tap(&mut rt, "c15-open-danger");
            run(&mut rt, 0.3);
            close(&mut rt);
            run(&mut rt, 0.4);
            assert!(
                element(&rt, "c15-danger-dialog").is_none(),
                "{route} closes it"
            );
            assert_eq!(
                focused(&rt).as_deref(),
                Some("c15-open-danger"),
                "{route} returns focus to the opener"
            );
        }
    }

    /// app/dialogs: "A dialog closed from the keyboard keeps the focus ring where focus lands
    /// once the last modal closes."
    #[test]
    fn a_keyboard_close_keeps_the_ring_on_the_opener() {
        let mut rt = mount("c15-openers-light");
        tab_to(&mut rt, "c15-open-basic");
        press(&mut rt, Key::Text(" ".into()), false);
        run(&mut rt, 0.3);
        press(&mut rt, Key::Escape, false);
        run(&mut rt, 0.4);
        assert!(focus_visible(&rt, "c15-open-basic"));
    }

    /// app/dialogs: "A dialog closed from the keyboard over a modal that stays open, such as
    /// Settings, keeps the ring on the control that gets focus."
    #[test]
    fn a_keyboard_close_over_a_dialog_keeps_the_ring_on_its_opener_there() {
        let mut rt = mount("c15-openers-light");
        tab_to(&mut rt, "c15-open-outer");
        press(&mut rt, Key::Text(" ".into()), false);
        run(&mut rt, 0.3);
        tab_to(&mut rt, "c15-outer-permissions");
        press(&mut rt, Key::Text(" ".into()), false);
        run(&mut rt, 0.3);
        assert_eq!(focused(&rt).as_deref(), Some("c15-inner-primary"));
        press(&mut rt, Key::Escape, false);
        run(&mut rt, 0.4);
        assert!(focus_visible(&rt, "c15-outer-permissions"));
    }
}
