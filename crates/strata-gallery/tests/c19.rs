//! C19's page in process: what the accessibility tree a script ends on doesn't show (scroll
//! offsets, the window losing focus, input-method text) and the large key map's cost. Each test
//! names the bullets it covers; the scripts in `tests/scripts/c19/` check the rest.

mod c19 {
    use std::time::Instant;

    use rune::{Env, Ime, Key, KeyEvent, Mods, Runtime};
    use strata_gallery::{Gallery, scenes};

    const FRAME: f64 = 1.0 / 60.0;
    const RESULTS: &str = "c19-reference-results";
    const SEARCH: &str = "c19-reference-search";

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
        run(&mut rt, 0.5);
        rt
    }

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        run(rt, 0.1);
    }

    fn plain(rt: &mut Runtime<Gallery>, key: Key) {
        press(rt, key, Mods::default());
    }

    fn ctrl(rt: &mut Runtime<Gallery>, c: char) {
        press(
            rt,
            Key::Command(c),
            Mods {
                cmd: true,
                ctrl: true,
                ..Mods::default()
            },
        );
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    /// The results' vertical scroll offset, as the accessibility tree reports it.
    fn offset(rt: &Runtime<Gallery>) -> f64 {
        fn find(n: &serde_json::Value) -> Option<f64> {
            if n["key"] == RESULTS {
                return n["scroll"][1].as_f64();
            }
            n["children"].as_array()?.iter().find_map(find)
        }
        find(&rt.accessibility_json()).unwrap_or_else(|| panic!("no {RESULTS} in the tree"))
    }

    fn mounted_rows(rt: &Runtime<Gallery>) -> usize {
        rt.elements()
            .into_iter()
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("c19-reference-row-"))
            })
            .count()
    }

    /// `app/shortcut-reference`: "In the results, arrows, j/k, Page Up, Page Down, Home, and End
    /// scroll the list" and "With the search field focused, Up, Down, Page Up, and Page Down
    /// scroll the results without moving focus".
    #[test]
    fn the_results_keys_scroll_the_list_and_the_search_field_scrolls_it_in_place() {
        let mut rt = mount("c19-reference-light");
        assert_eq!(focused(&rt).as_deref(), Some(SEARCH));
        plain(&mut rt, Key::Down);
        plain(&mut rt, Key::Down);
        let after_two = offset(&rt);
        assert!(after_two > 0.0, "Down in the search field scrolls");
        assert_eq!(focused(&rt).as_deref(), Some(SEARCH), "and keeps focus");
        plain(&mut rt, Key::PageDown);
        let paged = offset(&rt);
        assert!(paged > after_two, "Page Down in the field scrolls a page");
        plain(&mut rt, Key::PageUp);
        assert!(offset(&rt) < paged);

        ctrl(&mut rt, 'l');
        assert_eq!(focused(&rt).as_deref(), Some(RESULTS));
        plain(&mut rt, Key::Home);
        assert_eq!(offset(&rt), 0.0);
        for k in ["j", "j", "j"] {
            plain(&mut rt, Key::Text(k.into()));
        }
        let down = offset(&rt);
        assert!(down > 0.0, "j scrolls down");
        plain(&mut rt, Key::Text("k".into()));
        assert!(offset(&rt) < down, "k scrolls up");
        plain(&mut rt, Key::Down);
        plain(&mut rt, Key::Up);
        plain(&mut rt, Key::End);
        let end = offset(&rt);
        plain(&mut rt, Key::PageDown);
        assert_eq!(offset(&rt), end, "End reaches the bottom");
        plain(&mut rt, Key::Home);
        assert_eq!(offset(&rt), 0.0);
        assert_eq!(focused(&rt).as_deref(), Some(RESULTS));
    }

    /// `app/shortcut-reference`: "Opening the reference focuses … and scrolls the list to the
    /// top" and "Reopening the reference keeps the previous search text and selected category".
    #[test]
    fn reopening_scrolls_the_results_to_the_top_and_keeps_the_query() {
        let mut rt = mount("c19-reference-light");
        plain(&mut rt, Key::Text("e".into()));
        plain(&mut rt, Key::PageDown);
        assert!(offset(&rt) > 0.0);
        plain(&mut rt, Key::Escape);
        run(&mut rt, 0.5);
        assert!(!rt.state.c19.reference.is_open());
        plain(&mut rt, Key::Function(1));
        run(&mut rt, 0.5);
        assert!(rt.state.c19.reference.is_open());
        assert_eq!(offset(&rt), 0.0);
        assert_eq!(focused(&rt).as_deref(), Some(SEARCH));
        assert_eq!(rt.state.c19.reference.query(), "e");
    }

    /// `app/shortcut-reference`: "Focusing another application window leaves the reference
    /// open with its search text, category, and scroll position" (scripts have no window-focus
    /// event: Rune draft U-X1-1).
    #[test]
    fn the_window_losing_focus_leaves_the_reference_as_it_was() {
        let mut rt = mount("c19-reference-light");
        plain(&mut rt, Key::Text("i".into()));
        plain(&mut rt, Key::PageDown);
        let at = offset(&rt);
        rt.state
            .c19
            .reference
            .set_category(Some("Files and selection".into()));
        run(&mut rt, 0.1);
        rt.window_focus(false);
        run(&mut rt, 0.5);
        rt.window_focus(true);
        run(&mut rt, 0.2);
        let r = &rt.state.c19.reference;
        assert!(r.is_open());
        assert_eq!(r.query(), "i");
        assert_eq!(r.category().as_deref(), Some("Files and selection"));
        assert!(at > 0.0);
        assert_eq!(offset(&rt), at, "the scroll position stays");
    }

    /// `app/shortcut-reference`: "With fcitx5 … Chinese can be typed there" (the input method's
    /// own switch key is the input method's; scripts have no input-method event, U-C6-7).
    #[test]
    fn an_input_method_types_into_the_search() {
        let mut rt = mount("c19-reference-light");
        rt.ime(Ime::Preedit("zhong".into(), None));
        run(&mut rt, 0.1);
        rt.ime(Ime::Commit("中文".into()));
        run(&mut rt, 0.1);
        assert_eq!(rt.state.c19.reference.query(), "中文");
        assert!(rt.state.c19.reference.is_open());
    }

    /// The plan's performance check: with 5,000 rows, a keystroke builds only the rows on
    /// screen. Timings print with `--nocapture`; they aren't asserted.
    #[test]
    fn a_keystroke_in_a_5000_row_reference_builds_only_the_rows_on_screen() {
        let mut rt = mount("c19-reference-large-light");
        let before = mounted_rows(&rt);
        assert!(before > 0 && before < 120, "{before} rows mounted");
        let mut times = Vec::new();
        for c in ["1", "2", "3"] {
            let t = Instant::now();
            rt.key(KeyEvent {
                key: Key::Text(c.into()),
                mods: Mods::default(),
            });
            rt.frame(FRAME);
            times.push(t.elapsed());
            run(&mut rt, 0.1);
            assert!(
                mounted_rows(&rt) < 120,
                "{} rows mounted",
                mounted_rows(&rt)
            );
        }
        let t = Instant::now();
        for _ in 0..3 {
            rt.key(KeyEvent {
                key: Key::Backspace,
                mods: Mods::default(),
            });
            rt.frame(FRAME);
        }
        let clear = t.elapsed() / 3;
        println!("c19 perf: 5,000 rows, keystroke + frame {times:?}, backspace + frame {clear:?}");
        assert_eq!(rt.state.c19.reference.query(), "");
    }

    fn mods(shift: bool, ctrl: bool, alt: bool, logo: bool) -> Mods {
        Mods {
            shift,
            cmd: ctrl,
            ctrl,
            alt,
            logo,
        }
    }

    /// The review's chords: browsing shortcuts, editing chords with and without Shift, Alt and
    /// Super chords, function keys.
    fn chords() -> Vec<(&'static str, Key, Mods)> {
        let n = mods(false, false, false, false);
        let c = mods(false, true, false, false);
        let cs = mods(true, true, false, false);
        let s = mods(true, false, false, false);
        let a = mods(false, false, true, false);
        let l = mods(false, false, false, true);
        vec![
            ("Ctrl+A", Key::Command('a'), c),
            ("Ctrl+Z", Key::Command('z'), c),
            ("Ctrl+Shift+Z", Key::Command('z'), cs),
            ("Ctrl+Y", Key::Command('y'), c),
            ("Ctrl+Shift+A", Key::Command('a'), cs),
            ("Ctrl+Shift+C", Key::Command('c'), cs),
            ("Ctrl+Shift+X", Key::Command('x'), cs),
            ("Ctrl+Shift+Y", Key::Command('y'), cs),
            ("Ctrl+Shift+V", Key::Command('v'), cs),
            ("Ctrl+V", Key::Command('v'), c),
            ("Ctrl+Backspace", Key::Backspace, c),
            ("Ctrl+Delete", Key::Delete, c),
            ("Delete", Key::Delete, n),
            ("Shift+Delete", Key::Delete, s),
            ("Shift+Backspace", Key::Backspace, s),
            ("Ctrl+Shift+Delete", Key::Delete, cs),
            ("Ctrl+,", Key::Command(','), c),
            ("Ctrl+1", Key::Command('1'), c),
            ("Ctrl+2", Key::Command('2'), c),
            ("Ctrl+3", Key::Command('3'), c),
            ("Alt+d", Key::Text("d".into()), a),
            ("Alt+1", Key::Text("1".into()), a),
            ("Super+e", Key::Text("e".into()), l),
            ("Alt+Left", Key::Left, a),
            ("Alt+Home", Key::Home, a),
            ("Ctrl+Home", Key::Home, c),
            ("Ctrl+Shift+End", Key::End, cs),
            ("Shift+Left", Key::Left, s),
            ("Shift+Home", Key::Home, s),
            ("Enter", Key::Enter, n),
            ("Shift+Enter", Key::Enter, s),
            ("Ctrl+Enter", Key::Enter, c),
            ("Alt+Enter", Key::Enter, a),
            ("F2", Key::Function(2), n),
            ("F5", Key::Function(5), n),
            ("Insert", Key::Insert, n),
            ("Shift+Insert", Key::Insert, s),
            ("Ctrl+Insert", Key::Insert, c),
            ("Menu", Key::ContextMenu, n),
            ("Shift+F10", Key::Function(10), s),
            ("Ctrl+Tab", Key::Tab, c),
            ("Ctrl+Shift+Tab", Key::Tab, cs),
            ("Ctrl+PageDown", Key::PageDown, c),
            ("Shift+PageDown", Key::PageDown, s),
            ("Shift+Down", Key::Down, s),
            ("Ctrl+Up", Key::Up, c),
            ("Alt+Up", Key::Up, a),
            ("Ctrl+Space", Key::Text(" ".into()), c),
            ("Alt+Space", Key::Text(" ".into()), a),
            (
                "Ctrl+Alt+c",
                Key::Command('c'),
                mods(false, true, true, false),
            ),
            ("Ctrl+K", Key::Command('k'), c),
            ("Ctrl+Shift+K", Key::Command('k'), cs),
            ("Ctrl+H", Key::Command('h'), c),
            ("Ctrl+W", Key::Command('w'), c),
            ("Ctrl+T", Key::Command('t'), c),
            ("Ctrl+N", Key::Command('n'), c),
            ("Ctrl+Shift+N", Key::Command('n'), cs),
            ("Ctrl+=", Key::Command('='), c),
            ("Ctrl+-", Key::Command('-'), c),
            ("Ctrl+Shift+M", Key::Command('m'), cs),
            ("Backspace", Key::Backspace, n),
            ("Space", Key::Text(" ".into()), n),
            ("x", Key::Text("x".into()), n),
            ("Shift+X", Key::Text("X".into()), s),
        ]
    }

    /// `app/shortcut-reference`: "While the reference is open, Delete, Ctrl+V, and other
    /// browsing shortcuts do nothing to files" and "In the reference's search, a Ctrl, Alt, or
    /// Super chord the field does not use reaches nothing else". Every chord, from the search
    /// field, the results and the categories: none reaches the page's key hook (which logs any
    /// that does) while the reference is open.
    #[test]
    fn no_key_reaches_the_window_while_the_reference_is_open() {
        let mut leaks = Vec::new();
        for (area, to) in [
            ("search", None),
            ("results", Some('l')),
            ("categories", Some('b')),
        ] {
            for (name, key, m) in chords() {
                let mut rt = mount("c19-reference-light");
                rt.state.c19.reference.set_query("ab");
                run(&mut rt, 0.1);
                if let Some(c) = to {
                    ctrl(&mut rt, c);
                }
                rt.key(KeyEvent { key, mods: m });
                run(&mut rt, 0.2);
                if !rt.state.c19.leaks.is_empty() {
                    leaks.push(format!("{area}: {name} -> {:?}", rt.state.c19.leaks));
                }
            }
        }
        assert!(
            leaks.is_empty(),
            "keys reached the window:\n{}",
            leaks.join("\n")
        );
    }

    /// `app/shortcut-reference`: "With the search field focused, Up, Down, Page Up, and Page
    /// Down scroll the results" with Shift held too, and in the results Left and Right scroll as
    /// Up and Down do (Strata's `scroll_motion` and `search_scroll_key` ignore Shift).
    #[test]
    fn shifted_keys_and_left_and_right_scroll_the_results_as_strata_does() {
        let shift = mods(true, false, false, false);
        let mut rt = mount("c19-reference-light");
        press(&mut rt, Key::Down, shift);
        press(&mut rt, Key::Down, shift);
        let stepped = offset(&rt);
        assert!(stepped > 0.0, "Shift+Down in the search field scrolls");
        press(&mut rt, Key::PageDown, shift);
        assert!(
            offset(&rt) > stepped,
            "Shift+Page Down in the search field scrolls"
        );
        assert_eq!(focused(&rt).as_deref(), Some(SEARCH));

        ctrl(&mut rt, 'l');
        plain(&mut rt, Key::Home);
        plain(&mut rt, Key::Right);
        plain(&mut rt, Key::Right);
        let right = offset(&rt);
        assert!(right > 0.0, "Right in the results scrolls down");
        plain(&mut rt, Key::Left);
        assert!(offset(&rt) < right, "Left scrolls up");
        press(&mut rt, Key::End, shift);
        let end = offset(&rt);
        assert!(end > right, "Shift+End reaches the bottom");
        press(&mut rt, Key::Home, shift);
        assert_eq!(offset(&rt), 0.0, "Shift+Home the top");
    }

    /// A category whose section goes (new data while open) shows All, and is forgotten: the
    /// section coming back later doesn't choose it again.
    #[test]
    fn a_vanished_category_is_forgotten() {
        let mut rt = mount("c19-reference-light");
        rt.update(|g, _| g.c19.reference.set_category(Some("Preview media".into())));
        run(&mut rt, 0.1);
        rt.update(|g, _| {
            g.c19.data.sections.pop();
            g.c19.data.revision += 100;
        });
        run(&mut rt, 0.3);
        assert_eq!(rt.state.c19.reference.category(), None);
        rt.update(|g, _| g.c19.refresh());
        run(&mut rt, 0.3);
        assert_eq!(rt.state.c19.reference.category(), None, "not chosen again");
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("c19-reference-category-4"))
        );
    }

    fn node(rt: &Runtime<Gallery>, key: &str) -> serde_json::Value {
        fn find(n: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
            if n["key"] == key {
                return Some(n.clone());
            }
            n["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        find(&rt.accessibility_json(), key).unwrap_or_else(|| panic!("no {key} in the tree"))
    }

    /// Choosing a category is announced once: the tab says its count, so the announced result
    /// count stays silent until a query is typed.
    #[test]
    fn a_category_change_is_announced_once() {
        let mut rt = mount("c19-reference-light");
        ctrl(&mut rt, 'b');
        plain(&mut rt, Key::Down);
        assert_eq!(
            node(&rt, "c19-reference-category-1")["description"],
            "23 shortcuts"
        );
        let status = node(&rt, "c19-reference-status");
        assert!(
            status["name"].is_null(),
            "the status says nothing: {status}"
        );
        ctrl(&mut rt, 'f');
        plain(&mut rt, Key::Text("p".into()));
        assert_eq!(node(&rt, "c19-reference-status")["name"], "14 shortcuts");
    }

    fn in_tree(rt: &Runtime<Gallery>, key: &str) -> bool {
        fn find(n: &serde_json::Value, key: &str) -> bool {
            n["key"] == key
                || n["children"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|c| find(c, key)))
        }
        find(&rt.accessibility_json(), key)
    }

    /// The reference sits above everything else in the overlay layer: in the frame its button
    /// opens it, an armed chord's panel (which the next poll clears) is under the modal, out of
    /// the accessibility tree, not drawn over the dialog.
    #[test]
    fn the_reference_is_the_top_overlay() {
        let mut rt = mount("c19-tenxer-light");
        rt.update(|g, _| g.c19.chord.arm(scenes::c19::go_chord()));
        run(&mut rt, 0.1);
        assert!(in_tree(&rt, "c19-chord-panel"));
        let (x, y) = rt.find("c19-footer-shortcuts").expect("the F1 button");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        assert!(rt.state.c19.reference.is_open());
        assert!(in_tree(&rt, "c19-reference-dialog"));
        assert!(
            !in_tree(&rt, "c19-chord-panel"),
            "the chord panel is above the modal"
        );
    }
}
