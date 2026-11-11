//! C20a's page in process: what each control reports (live, commit, cancel), a press on the
//! track, Home and End, assistive actions, and the field menu holding a field's commit. The
//! scripts in `tests/scripts/c20a/` check the bullets' end states.

mod c20a {
    use rune::accesskit::{Action, ActionData};
    use rune::runtime::ElementInfo;
    use rune::{Env, Key, KeyEvent, Mods, Runtime, WheelEvent};
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

    fn centre(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        let r = element(rt, key).rect;
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    fn click(rt: &mut Runtime<Gallery>, (x, y): (f32, f32)) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(FRAME);
        rt.frame(FRAME);
    }

    fn type_text(rt: &mut Runtime<Gallery>, text: &str) {
        for c in text.chars() {
            press(rt, Key::Text(c.to_string()), Mods::default());
        }
    }

    fn heard(rt: &Runtime<Gallery>) -> Vec<&str> {
        rt.state.c20a.heard.iter().map(String::as_str).collect()
    }

    /// Where Frame's knob sits for `px`: the slider's track less half a knob at each end.
    fn frame_x(rt: &Runtime<Gallery>, px: f32) -> (f32, f32) {
        let s = element(rt, "c20a-frame").rect;
        let knob = element(rt, "c20a-frame-knob").rect[2];
        let x = s[0] + knob / 2.0 + px / 12.0 * (s[2] - knob);
        (x, s[1] + s[3] / 2.0)
    }

    #[test]
    fn a_drag_reports_live_values_then_one_commit() {
        let mut rt = mount("c20a-controls-light");
        let (x, y) = centre(&rt, "c20a-frame-knob");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        for px in [1.0, 2.0, 3.0] {
            let (to, _) = frame_x(&rt, px);
            rt.pointer_move(to, y);
            rt.frame(FRAME);
        }
        assert_eq!(heard(&rt), ["Frame Live 1", "Frame Live 2", "Frame Live 3"]);
        rt.pointer_up(frame_x(&rt, 3.0).0, y);
        rt.frame(FRAME);
        assert_eq!(heard(&rt).last(), Some(&"Frame Commit 3"));
        assert_eq!(rt.state.c20a.frame.value(), 3.0);
        assert!(!rt.state.c20a.frame.dragging());
    }

    #[test]
    fn escape_cancels_a_drag_back_to_its_start_and_keeps_the_page() {
        let mut rt = mount("c20a-controls-light");
        let (x, y) = centre(&rt, "c20a-frame-knob");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_move(frame_x(&rt, 6.0).0, y);
        rt.frame(FRAME);
        assert_eq!(rt.state.c20a.frame.value(), 6.0);
        press(&mut rt, Key::Escape, Mods::default());
        rt.pointer_up(frame_x(&rt, 6.0).0, y);
        rt.frame(FRAME);
        assert_eq!(heard(&rt), ["Frame Live 6", "Frame Cancel 0"]);
        assert_eq!(rt.state.c20a.frame.value(), 0.0);
        assert_eq!(
            rt.state.page,
            Some("c20a"),
            "the Escape went to the drag, not the gallery"
        );
    }

    /// Whether Frame's knob shows the engaged look (it grows while held).
    fn frame_engaged(rt: &Runtime<Gallery>) -> bool {
        element(rt, "c20a-frame-knob").scale > 1.0
    }

    #[test]
    fn a_held_press_engages_the_knob_until_release_or_cancel() {
        let mut rt = mount("c20a-controls-light");
        assert!(!frame_engaged(&rt));
        let (x, y) = centre(&rt, "c20a-frame-knob");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        assert!(frame_engaged(&rt), "a press on the knob engages it");
        assert!(heard(&rt).is_empty(), "before it moves");
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        assert!(!frame_engaged(&rt), "a release lets it go");

        let at = frame_x(&rt, 9.0);
        rt.pointer_move(at.0, at.1);
        rt.pointer_down(at.0, at.1);
        rt.frame(FRAME);
        assert!(frame_engaged(&rt), "a press on the track engages it");
        press(&mut rt, Key::Escape, Mods::default());
        assert!(!frame_engaged(&rt), "Escape lets it go");
        rt.pointer_up(at.0, at.1);
        rt.frame(FRAME);
        assert!(!frame_engaged(&rt));
    }

    #[test]
    fn a_press_on_the_track_jumps_there_and_commits_on_release() {
        let mut rt = mount("c20a-controls-light");
        let at = frame_x(&rt, 9.0);
        click(&mut rt, at);
        assert_eq!(heard(&rt), ["Frame Live 9", "Frame Commit 9"]);
    }

    #[test]
    fn home_and_end_take_a_focused_slider_to_its_ends() {
        let mut rt = mount("c20a-controls-light");
        let at = centre(&rt, "c20a-thumbs-knob");
        click(&mut rt, at);
        press(&mut rt, Key::End, Mods::default());
        assert_eq!(rt.state.c20a.thumbs.value(), 256.0);
        press(&mut rt, Key::Home, Mods::default());
        assert_eq!(rt.state.c20a.thumbs.value(), 32.0);
        assert_eq!(
            heard(&rt),
            ["Thumbnail size Commit 256", "Thumbnail size Commit 32"]
        );
    }

    #[test]
    fn page_keys_with_a_modifier_pass_the_slider_by() {
        let mut rt = mount("c20a-controls-light");
        let at = centre(&rt, "c20a-frame-knob");
        click(&mut rt, at);
        for mods in [
            Mods {
                shift: true,
                ..Mods::default()
            },
            Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
            Mods {
                alt: true,
                ..Mods::default()
            },
        ] {
            press(&mut rt, Key::PageUp, mods);
        }
        assert_eq!(rt.state.c20a.frame.value(), 0.0);
        press(&mut rt, Key::PageUp, Mods::default());
        assert_eq!(heard(&rt), ["Frame Commit 4"], "a plain Page Up pages 4 px");
    }

    #[test]
    fn assistive_technology_steps_the_spin_button_and_sets_the_slider() {
        let mut rt = mount("c20a-controls-light");
        assert!(rt.accessibility_action("c20a-workers", Action::Increment, None));
        rt.frame(FRAME);
        assert_eq!(rt.state.c20a.workers.value(), 5.0);
        assert_eq!(rt.state.c20a.workers.edit().text, "5");
        assert!(rt.accessibility_action(
            "c20a-thumbs",
            Action::SetValue,
            Some(ActionData::NumericValue(128.0))
        ));
        rt.frame(FRAME);
        assert_eq!(rt.state.c20a.thumbs.value(), 128.0);
        assert_eq!(heard(&rt), ["Workers 5", "Thumbnail size Commit 128"]);
        assert!(
            !rt.accessibility_action("c20a-workers", Action::Focus, None),
            "the keyboard belongs to the field inside the spin button"
        );
    }

    #[test]
    fn a_stepper_press_keeps_the_keyboard_in_the_field() {
        let mut rt = mount("c20a-controls-light");
        let at = centre(&rt, "c20a-workers-input");
        click(&mut rt, at);
        assert!(rt.state.c20a.workers.edit().focused);
        let at = centre(&rt, "c20a-workers-increase");
        click(&mut rt, at);
        assert!(rt.state.c20a.workers.edit().focused);
        assert_eq!(heard(&rt), ["Workers 5"]);
    }

    fn wheel(rt: &mut Runtime<Gallery>, key: &str, dx: f32, dy: f32, mods: Mods) {
        let (x, y) = centre(rt, key);
        rt.pointer_move(x, y);
        rt.wheel_event(WheelEvent::new(dx, dy).at(x, y).mods(mods));
        for _ in 0..3 {
            rt.frame(FRAME);
        }
    }

    /// A sideways notch (Shift+wheel on Linux, a tilt wheel), and a notch with Ctrl (the text
    /// size), Ctrl+Shift (Strata's thumbnail size) or Alt, go past both controls to the page.
    #[test]
    fn a_sideways_or_modified_wheel_passes_both_controls_by() {
        let ctrl = Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        };
        let cases = [
            ((-40.0, 0.0), Mods::default()),
            ((40.0, 0.0), Mods::default()),
            ((0.0, 40.0), ctrl),
            (
                (0.0, 40.0),
                Mods {
                    shift: true,
                    ..ctrl
                },
            ),
            (
                (0.0, -40.0),
                Mods {
                    alt: true,
                    ..Mods::default()
                },
            ),
        ];
        for key in ["c20a-thumbs", "c20a-workers"] {
            for ((dx, dy), mods) in cases {
                let mut rt = mount("c20a-controls-light");
                wheel(&mut rt, key, dx, dy, mods);
                assert_eq!(
                    (rt.state.c20a.thumbs.value(), rt.state.c20a.workers.value()),
                    (64.0, 4.0),
                    "{key} ({dx}, {dy}) {mods:?} moved nothing"
                );
                assert!(heard(&rt).is_empty(), "{key} ({dx}, {dy}) {mods:?}");
                assert_eq!(
                    rt.state.c20a.log, "Page wheel",
                    "{key} ({dx}, {dy}) {mods:?} reached the page"
                );
            }
        }
    }

    /// The density sliders take no wheel: it scrolls the Appearance menu they sit in.
    #[test]
    fn a_wheel_over_a_density_slider_scrolls_its_menu() {
        let mut rt = mount("c20a-controls-light");
        let before = element(&rt, "c20a-frame").rect[1];
        wheel(&mut rt, "c20a-frame", 0.0, -40.0, Mods::default());
        for _ in 0..30 {
            rt.frame(FRAME);
        }
        assert!(
            element(&rt, "c20a-frame").rect[1] < before,
            "the menu scrolled the slider up"
        );
        assert_eq!(rt.state.c20a.frame.value(), 0.0);
        assert!(heard(&rt).is_empty());
    }

    #[test]
    fn typed_text_waits_while_the_field_menu_is_open_then_commits_on_the_blur() {
        let mut rt = mount("c20a-controls-light");
        let at = centre(&rt, "c20a-workers-input");
        click(&mut rt, at);
        press(&mut rt, Key::End, Mods::default());
        press(&mut rt, Key::Backspace, Mods::default());
        type_text(&mut rt, "9");
        rt.secondary_down(at.0, at.1);
        rt.secondary_up(at.0, at.1);
        for _ in 0..3 {
            rt.frame(FRAME);
        }
        assert!(rt.state.c20a.fields.is_open(), "the field's menu opened");
        assert!(
            !rt.state.c20a.workers.edit().focused,
            "and has the keyboard"
        );
        assert_eq!(
            rt.state.c20a.workers.value(),
            4.0,
            "losing the keyboard to it commits nothing"
        );
        assert_eq!(rt.state.c20a.workers.edit().text, "9");
        press(&mut rt, Key::Escape, Mods::default());
        assert!(!rt.state.c20a.fields.is_open());
        assert!(
            rt.state.c20a.workers.edit().focused,
            "the keyboard is back in the field"
        );
        assert_eq!(rt.state.c20a.workers.edit().text, "9");
        press(&mut rt, Key::Tab, Mods::default());
        assert_eq!(rt.state.c20a.workers.value(), 9.0);
        assert_eq!(heard(&rt).last(), Some(&"Workers 9"));
    }
}
