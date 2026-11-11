//! C20b's page in process: what the picker reports (live, commit, cancel), Escape on a drag and
//! in the popover, focus as the popover opens and closes, a press outside, the hex field's
//! blur, assistive actions, the disabled state and the cost of a hue drag. The scripts in
//! `tests/scripts/c20b/` check the end states.

mod c20b {
    use std::time::Instant;

    use rune::accesskit::Action;
    use rune::runtime::ElementInfo;
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::controls::color_picker::Srgba;
    use strata_gallery::scenes::c20b::Pick;
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
        find(rt, key).unwrap_or_else(|| panic!("no {key}"))
    }

    fn find(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
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

    /// A point `fx`, `fy` of the way across `key`'s box.
    fn at(rt: &Runtime<Gallery>, key: &str, fx: f32, fy: f32) -> (f32, f32) {
        let r = element(rt, key).rect;
        (r[0] + fx * r[2], r[1] + fy * r[3])
    }

    fn click(rt: &mut Runtime<Gallery>, (x, y): (f32, f32)) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        rt.frame(FRAME);
    }

    /// A click `fx`, `fy` of the way across `key`'s box.
    fn tap(rt: &mut Runtime<Gallery>, key: &str, fx: f32, fy: f32) {
        let p = at(rt, key, fx, fy);
        click(rt, p);
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(FRAME);
        rt.frame(FRAME);
    }

    fn heard(rt: &Runtime<Gallery>) -> Vec<&str> {
        rt.state.c20b.heard.iter().map(String::as_str).collect()
    }

    fn kinds(rt: &Runtime<Gallery>) -> Vec<&str> {
        heard(rt)
            .into_iter()
            .map(|l| l.split(' ').nth_back(1).unwrap_or(""))
            .collect()
    }

    #[test]
    fn a_plane_drag_reports_live_then_one_commit() {
        let mut rt = mount("c20b-opaque-light");
        let (x, y) = at(&rt, "c20b-folder-plane", 0.5, 0.5);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        for i in 1..=5 {
            rt.pointer_move(x + 10.0 * i as f32, y - 5.0 * i as f32);
            rt.frame(FRAME);
        }
        rt.pointer_up(x + 50.0, y - 25.0);
        rt.frame(FRAME);
        let k = kinds(&rt);
        assert_eq!(k.len(), 7, "{:?}", heard(&rt));
        assert!(k[..6].iter().all(|&c| c == "Live"), "{:?}", heard(&rt));
        assert_eq!(k[6], "Commit");
        let value = rt.state.c20b.folder.value();
        assert!(heard(&rt)[6].ends_with(&value.hex()));
        assert_eq!(
            rt.state.c20b.folder.edit().text,
            value.hex(),
            "the hex follows"
        );
        assert_eq!(focused(&rt).as_deref(), Some("c20b-folder-plane"));
    }

    #[test]
    fn escape_cancels_a_drag_back_to_its_start_and_the_page_stays() {
        let mut rt = mount("c20b-opaque-light");
        let start = rt.state.c20b.folder.value();
        let (x, y) = at(&rt, "c20b-folder-hue", 0.2, 0.5);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_move(x + 40.0, y);
        rt.frame(FRAME);
        assert_ne!(rt.state.c20b.folder.value(), start);
        press(&mut rt, Key::Escape, Mods::default());
        assert_eq!(rt.state.c20b.folder.value(), start);
        assert_eq!(kinds(&rt).last(), Some(&"Cancel"));
        assert!(!rt.state.c20b.folder.dragging());
        rt.pointer_up(x + 40.0, y);
        rt.frame(FRAME);
        assert_eq!(rt.state.page, Some("c20b"), "the cancel took the Escape");
    }

    #[test]
    fn the_popover_opens_on_the_plane_and_escape_puts_the_colour_back() {
        let mut rt = mount("c20b-opaque-light");
        let start = rt.state.c20b.accent.value();
        tap(&mut rt, "c20b-accent-button", 0.5, 0.5);
        assert!(rt.state.c20b.accent.is_open());
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-plane"));
        press(&mut rt, Key::Right, Mods::default());
        press(&mut rt, Key::Tab, Mods::default());
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-hue"));
        press(&mut rt, Key::Right, Mods::default());
        assert_ne!(rt.state.c20b.accent.value(), start);
        press(&mut rt, Key::Escape, Mods::default());
        assert!(!rt.state.c20b.accent.is_open());
        assert_eq!(rt.state.c20b.accent.value(), start);
        let h = heard(&rt);
        assert_eq!(h[h.len() - 2], format!("Accent Cancel {start}"));
        assert_eq!(h[h.len() - 1], "Accent closed, changed false");
        assert_eq!(rt.state.c20b.recent.len(), 3, "nothing remembered");
        rt.frame(FRAME);
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-button"));
        assert_eq!(rt.state.page, Some("c20b"));
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::default()
        }
    }

    /// Rune's modal dismisses on Escape with Shift held, as on bare Escape.
    #[test]
    fn shift_escape_in_the_popover_puts_the_colour_back_as_escape_does() {
        let mut rt = mount("c20b-opaque-light");
        let start = rt.state.c20b.accent.value();
        tap(&mut rt, "c20b-accent-button", 0.5, 0.5);
        press(&mut rt, Key::Right, Mods::default());
        assert_ne!(rt.state.c20b.accent.value(), start);
        press(&mut rt, Key::Escape, shift());
        assert!(!rt.state.c20b.accent.is_open());
        assert_eq!(rt.state.c20b.accent.value(), start);
        let h = heard(&rt);
        assert_eq!(h[h.len() - 2], format!("Accent Cancel {start}"));
        assert_eq!(h[h.len() - 1], "Accent closed, changed false");
        rt.frame(FRAME);
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-button"));
    }

    #[test]
    fn shift_escape_in_the_popover_field_reverts_typing_then_puts_the_colour_back() {
        let mut rt = mount("c20b-popover-light");
        let start = rt.state.c20b.accent.value();
        tap(&mut rt, "c20b-accent-hex", 0.5, 0.5);
        rt.update(|g, _| g.c20b.accent.edit_mut().replace_text("#123"));
        rt.frame(FRAME);
        press(&mut rt, Key::Escape, shift());
        assert!(
            rt.state.c20b.accent.is_open(),
            "the first Shift+Escape reverts the text"
        );
        assert_eq!(rt.state.c20b.accent.edit().text, start.hex());
        press(&mut rt, Key::Escape, shift());
        assert!(!rt.state.c20b.accent.is_open());
        assert_eq!(rt.state.c20b.accent.value(), start);
        assert_eq!(heard(&rt), ["Accent closed, changed false"]);
    }

    /// A popover's Cancel comes after the Commits its keys, swatches and releases sent: the
    /// host saves the opening colour it reports.
    #[test]
    fn the_host_saves_the_opening_colour_a_popover_cancel_reports() {
        let mut rt = mount("c20b-opaque-light");
        let start = rt.state.c20b.accent.value();
        tap(&mut rt, "c20b-accent-button", 0.5, 0.5);
        press(&mut rt, Key::Right, Mods::default());
        let moved = rt.state.c20b.accent.value();
        press(&mut rt, Key::Escape, Mods::default());
        assert_eq!(
            rt.state.c20b.saved,
            [(Pick::Accent, moved), (Pick::Accent, start)]
        );
    }

    #[test]
    fn the_plane_passes_shift_ctrl_super_and_cmd_arrows_on() {
        let mut rt = mount("c20b-popover-light");
        let start = rt.state.c20b.accent.hsv();
        let held = [
            shift(),
            Mods {
                ctrl: true,
                ..Mods::default()
            },
            Mods {
                logo: true,
                ..Mods::default()
            },
            Mods {
                cmd: true,
                ..Mods::default()
            },
        ];
        for mods in held {
            for key in [Key::Right, Key::Up] {
                press(&mut rt, key, mods);
                assert_eq!(rt.state.c20b.accent.hsv(), start, "{mods:?}");
            }
        }
        assert!(heard(&rt).is_empty(), "{:?}", heard(&rt));
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-plane"));
    }

    /// Keys queued in one frame reach the plane and a swatch before the poll notices the
    /// field's blur.
    #[test]
    fn typed_hex_commits_before_a_plane_key_or_a_swatch() {
        let typed = |rt: &mut Runtime<Gallery>| {
            tap(rt, "c20b-folder-hex", 0.5, 0.5);
            rt.update(|g, _| g.c20b.folder.edit_mut().replace_text("#336699"));
            rt.frame(FRAME);
        };
        let mut rt = mount("c20b-opaque-light");
        typed(&mut rt);
        // Back past the hue strip to the plane, then a step right.
        for key in [Key::Tab, Key::Tab, Key::Right] {
            let mods = if key == Key::Tab {
                shift()
            } else {
                Mods::default()
            };
            rt.key(KeyEvent { key, mods });
        }
        rt.frame(FRAME);
        rt.frame(FRAME);
        assert_eq!(focused(&rt).as_deref(), Some("c20b-folder-plane"));
        let h = heard(&rt);
        assert_eq!(h[0], "Folder color Commit #336699", "{h:?}");
        assert_eq!(h.len(), 2, "{h:?}");
        assert_eq!(
            h[1],
            format!("Folder color Commit {}", rt.state.c20b.folder.value())
        );
        assert_eq!(rt.state.c20b.folder.hsv().h.round(), 210.0, "the typed hue");

        let mut rt = mount("c20b-opaque-light");
        typed(&mut rt);
        // On to the first preset, Red, and a press on it.
        rt.key(KeyEvent {
            key: Key::Tab,
            mods: Mods::default(),
        });
        rt.key(KeyEvent {
            key: Key::Text(" ".into()),
            mods: Mods::default(),
        });
        rt.frame(FRAME);
        rt.frame(FRAME);
        assert_eq!(focused(&rt).as_deref(), Some("c20b-folder-swatch-0-0"));
        assert_eq!(
            heard(&rt),
            ["Folder color Commit #336699", "Folder color Commit #e5484d"]
        );
    }

    #[test]
    fn disabling_a_picker_closes_its_popover_for_good() {
        let mut rt = mount("c20b-opaque-light");
        tap(&mut rt, "c20b-accent-button", 0.5, 0.5);
        assert!(rt.state.c20b.accent.is_open());
        rt.update(|g, _| g.c20b.disabled = true);
        for _ in 0..3 {
            rt.frame(FRAME);
        }
        assert!(!rt.state.c20b.accent.is_open());
        assert_eq!(heard(&rt), ["Accent closed, changed false"]);
        rt.update(|g, _| g.c20b.disabled = false);
        for _ in 0..30 {
            rt.frame(FRAME);
        }
        assert!(!rt.state.c20b.accent.is_open());
        assert!(
            find(&rt, "c20b-accent-plane").is_none_or(|e| e.exiting),
            "re-enabling doesn't bring it back"
        );
    }

    #[test]
    fn a_press_outside_keeps_the_colour_and_remembers_it() {
        let mut rt = mount("c20b-opaque-light");
        tap(&mut rt, "c20b-accent-button", 0.5, 0.5);
        tap(&mut rt, "c20b-accent-swatch-0-0", 0.5, 0.5);
        let red = Srgba::rgb(0xe5, 0x48, 0x4d);
        assert_eq!(rt.state.c20b.accent.value(), red);
        click(&mut rt, (900.0, 600.0));
        assert!(!rt.state.c20b.accent.is_open());
        assert_eq!(rt.state.c20b.accent.value(), red);
        assert_eq!(heard(&rt).last(), Some(&"Accent closed, changed true"));
        assert_eq!(rt.state.c20b.recent[0], red);
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-button"));
        for _ in 0..30 {
            rt.frame(FRAME);
        }
        assert!(
            find(&rt, "c20b-accent-plane").is_none(),
            "gone once it has faded"
        );
    }

    #[test]
    fn escape_in_the_hex_field_reverts_typing_then_closes_the_popover() {
        let mut rt = mount("c20b-popover-light");
        let start = rt.state.c20b.accent.value();
        tap(&mut rt, "c20b-accent-hex", 0.5, 0.5);
        assert_eq!(focused(&rt).as_deref(), Some("c20b-accent-hex"));
        press(&mut rt, Key::Text("x".into()), Mods::default());
        assert!(rt.state.c20b.accent.invalid());
        press(&mut rt, Key::Escape, Mods::default());
        assert!(
            rt.state.c20b.accent.is_open(),
            "the first Escape reverts the text"
        );
        assert_eq!(rt.state.c20b.accent.edit().text, start.hex());
        press(&mut rt, Key::Escape, Mods::default());
        assert!(!rt.state.c20b.accent.is_open());
        assert_eq!(rt.state.c20b.accent.value(), start);
    }

    #[test]
    fn typed_hex_commits_when_the_field_loses_the_keyboard() {
        let mut rt = mount("c20b-opaque-light");
        tap(&mut rt, "c20b-folder-hex", 0.5, 0.5);
        rt.update(|g, _| g.c20b.folder.edit_mut().replace_text("#123"));
        rt.frame(FRAME);
        assert!(heard(&rt).is_empty());
        press(&mut rt, Key::Tab, Mods::default());
        rt.frame(FRAME);
        assert_ne!(focused(&rt).as_deref(), Some("c20b-folder-hex"));
        assert_eq!(rt.state.c20b.folder.value(), Srgba::rgb(0x11, 0x22, 0x33));
        assert_eq!(heard(&rt), ["Folder color Commit #112233"]);
        assert_eq!(rt.state.c20b.folder.edit().text, "#112233");
    }

    #[test]
    fn typed_hex_commits_before_a_press_on_the_plane_uses_its_hue() {
        let mut rt = mount("c20b-opaque-light");
        tap(&mut rt, "c20b-folder-hex", 0.5, 0.5);
        rt.update(|g, _| g.c20b.folder.edit_mut().replace_text("#00f"));
        rt.frame(FRAME);
        tap(&mut rt, "c20b-folder-plane", 0.5, 0.5);
        assert_eq!(rt.state.c20b.folder.hsv().h, 240.0, "the typed blue's hue");
        let h = heard(&rt);
        assert_eq!(h[0], "Folder color Commit #0000ff");
        let c = rt.state.c20b.folder.value();
        assert!(c.b > c.r && c.b > c.g, "{c}");
    }

    #[test]
    fn assistive_technology_steps_and_sets_the_strips() {
        let mut rt = mount("c20b-translucent-light");
        let h = rt.state.c20b.glass.hsv().h.round();
        rt.accessibility_action("c20b-glass-hue", Action::Increment, None);
        rt.frame(FRAME);
        assert_eq!(rt.state.c20b.glass.hsv().h, h + 1.0);
        rt.accessibility_action("c20b-glass-alpha", Action::Decrement, None);
        rt.frame(FRAME);
        assert_eq!(rt.state.c20b.glass.value().a, 125, "50% down to 49%");
        assert_eq!(kinds(&rt), ["Commit", "Commit"]);
    }

    #[test]
    fn a_disabled_picker_hears_nothing() {
        let mut rt = mount("c20b-disabled-light");
        let start = rt.state.c20b.folder.value();
        tap(&mut rt, "c20b-folder-plane", 0.3, 0.3);
        tap(&mut rt, "c20b-folder-swatch-0-0", 0.5, 0.5);
        tap(&mut rt, "c20b-accent-button", 0.5, 0.5);
        press(&mut rt, Key::Tab, Mods::default());
        assert_eq!(rt.state.c20b.folder.value(), start);
        assert!(!rt.state.c20b.accent.is_open());
        assert!(heard(&rt).is_empty());
        assert!(
            focused(&rt).is_none_or(|k| !k.starts_with("c20b-")),
            "{:?}",
            focused(&rt)
        );
    }

    #[test]
    fn a_right_click_on_the_hex_field_opens_the_field_menu_and_holds_its_commit() {
        let mut rt = mount("c20b-opaque-light");
        tap(&mut rt, "c20b-folder-hex", 0.5, 0.5);
        rt.update(|g, _| g.c20b.folder.edit_mut().replace_text("#123"));
        rt.frame(FRAME);
        let (x, y) = at(&rt, "c20b-folder-hex", 0.5, 0.5);
        rt.pointer_move(x, y);
        rt.secondary_down(x, y);
        rt.frame(FRAME);
        rt.secondary_up(x, y);
        rt.frame(FRAME);
        rt.frame(FRAME);
        assert!(rt.state.c20b.fields.is_open());
        assert_eq!(
            rt.state.c20b.folder.edit().text,
            "#123",
            "no commit while the menu is open"
        );
        assert!(heard(&rt).is_empty());
    }

    /// Perf: a hue drag runs the page's view on each move. Prints each move's cost (the move
    /// and its frame); asserts each move commits once.
    #[test]
    fn a_hue_drag_commits_once_a_move() {
        let mut rt = mount("c20b-opaque-light");
        let (x0, y) = at(&rt, "c20b-folder-hue", 0.05, 0.5);
        let w = element(&rt, "c20b-folder-hue").rect[2] * 0.9;
        rt.pointer_move(x0, y);
        rt.pointer_down(x0, y);
        rt.frame(FRAME);
        let moves = 200;
        let c0 = rune::commit_generation();
        let mut times = Vec::new();
        for i in 1..=moves {
            let t = Instant::now();
            rt.pointer_move(x0 + w * i as f32 / moves as f32, y);
            rt.frame(FRAME);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let commits = rune::commit_generation() - c0;
        rt.pointer_up(x0 + w, y);
        rt.frame(FRAME);
        times.sort_by(f64::total_cmp);
        let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        eprintln!(
            "c20b perf: {moves} hue moves, {commits} commits, move+frame p50 {:.2} ms, p95 {:.2} \
             ms, max {:.2} ms",
            p(0.5),
            p(0.95),
            p(1.0)
        );
        assert!(
            commits <= moves as u64,
            "{commits} commits for {moves} moves"
        );
        assert_eq!(kinds(&rt).last(), Some(&"Commit"));
    }
}
