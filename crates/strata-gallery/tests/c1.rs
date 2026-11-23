// SPDX-License-Identifier: MIT

#[expect(
    dead_code,
    reason = "c1 uses only the gallery command and the scripts folder"
)]
#[path = "goldens/support.rs"]
mod support;

mod c1 {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::collection::element::{
        Ctx, Effect, Flow, Intent, PointerController, PointerInput, PointerPhase,
    };
    use strata_gallery::scenes::c1::{WARMED, WARMUP};
    use strata_gallery::{Gallery, scenes};

    use super::support;

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
        run(&mut rt, 0.5);
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
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let [x, y, w, h] = element(rt, key)
            .unwrap_or_else(|| panic!("no element {key}"))
            .visible;
        rt.pointer_down(x + 40.0, y + h / 2.0);
        rt.pointer_up(x + 40.0, y + h / 2.0);
        let _ = w;
        run(rt, 0.2);
    }

    fn key(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        run(rt, 0.3);
    }

    fn alt() -> Mods {
        Mods {
            alt: true,
            ..Mods::default()
        }
    }

    fn cmd() -> Mods {
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        }
    }

    /// Scrolls Photos about 300 rows down.
    fn scroll_photos(rt: &mut Runtime<Gallery>) {
        for _ in 0..20 {
            rt.wheel(480.0, 360.0, -400.0);
            rt.frame(FRAME);
        }
        run(rt, 0.5);
    }

    #[test]
    fn keyboard_navigation_holds_row_hover_until_the_pointer_moves() {
        let mut rt = mount("c1-ten-rows-light");
        let [x, y, _, h] = element(&rt, "row-3").unwrap().visible;
        rt.pointer_move(x + 300.0, y + h / 2.0);
        run(&mut rt, 0.3);
        assert!(rt.state.c1.coll.row_hover());

        rt.update(|g, _| g.c1.coll.keyboard_moved());
        run(&mut rt, 0.3);
        assert!(
            !rt.state.c1.coll.row_hover(),
            "keyboard navigation turns it off"
        );
        rt.wheel(480.0, 300.0, -10.0);
        run(&mut rt, 0.3);
        assert!(
            !rt.state.c1.coll.row_hover(),
            "a scroll under a parked pointer doesn't"
        );

        rt.pointer_move(x + 302.0, y + h / 2.0);
        run(&mut rt, 0.3);
        assert!(
            rt.state.c1.coll.row_hover(),
            "the pointer moving brings it back"
        );
    }

    #[test]
    fn a_live_update_keeps_the_selected_row_where_it_shows() {
        let mut rt = mount("c1-photos-light");
        scroll_photos(&mut rt);
        tap(&mut rt, "row-10310");
        let before = element(&rt, "row-10310").unwrap().visible;
        key(&mut rt, Key::Command('i'), cmd());
        assert_eq!(rt.state.c1.listing.entries().len(), 2_003);
        assert_eq!(element(&rt, "row-10310").unwrap().visible, before);
        assert!(rt.state.c1.coll.selection.contains(
            &rt.state.c1.listing,
            strata_components::contract::EntryId::new(10_310)
        ));
    }

    #[test]
    fn a_refresh_keeps_the_cursor_row_where_it_shows() {
        let mut rt = mount("c1-photos-light");
        rt.update(|g, tx| {
            let d = &mut g.c1;
            d.coll.selection.select_only(&d.listing, 1_999);
            d.coll
                .reveal(strata_gallery::scenes::c1::SCOPE, &d.listing, 1_999, tx);
        });
        run(&mut rt, 1.0);
        let before = element(&rt, "row-11999")
            .expect("the last row shows")
            .visible;
        key(&mut rt, Key::Function(5), Mods::default());
        assert_eq!(element(&rt, "row-11999").unwrap().visible, before);
    }

    #[test]
    fn a_restore_in_another_layout_scrolls_the_cursor_into_view() {
        let mut rt = mount("c1-photos-light");
        scroll_photos(&mut rt);
        tap(&mut rt, "row-10310");
        let left_at = rt.state.c1.coll.scroll_offset();
        key(&mut rt, Key::Left, alt());
        key(&mut rt, Key::Right, alt());
        assert_eq!(
            rt.state.c1.coll.scroll_offset(),
            left_at,
            "same layout: same offset"
        );

        // Scroll elsewhere, leave, switch the layout, come back.
        for _ in 0..10 {
            rt.wheel(480.0, 360.0, -400.0);
            rt.frame(FRAME);
        }
        run(&mut rt, 0.5);
        key(&mut rt, Key::Left, alt());
        key(&mut rt, Key::Command('2'), cmd());
        key(&mut rt, Key::Right, alt());
        let row = element(&rt, "row-10310").expect("the cursor's row is mounted");
        assert!(row.visible[3] > 20.0, "and shows");
        assert_ne!(rt.state.c1.coll.scroll_offset(), left_at);
    }

    #[test]
    fn a_press_before_a_slow_load_completes_cancels_the_restore() {
        for press in [false, true] {
            let mut rt = mount("c1-photos-light");
            tap(&mut rt, "row-10005");
            rt.update(|g, _| g.c1.slow_load = Some(0.5));
            key(&mut rt, Key::Left, alt());
            key(&mut rt, Key::Right, alt());
            assert!(rt.state.c1.loading.is_some(), "Photos loads slowly");
            if press {
                rt.pointer_down(480.0, 300.0);
                rt.pointer_up(480.0, 300.0);
            }
            run(&mut rt, 1.0);
            let d = &rt.state.c1;
            let cursor = d.coll.selection.cursor().map(|id| id.get());
            if press {
                assert_eq!(
                    cursor,
                    Some(10_000),
                    "the first-visit pick, not the restore"
                );
            } else {
                assert_eq!(cursor, Some(10_005), "the restore applied once complete");
            }
        }
    }

    fn text_of(rt: &Runtime<Gallery>, key: &str) -> String {
        element(rt, key).and_then(|e| e.text).unwrap_or_default()
    }

    /// The page shows the last intent: the default click finishes with `Clicked`.
    #[test]
    fn the_page_shows_the_last_intent() {
        let mut rt = mount("c1-ten-rows-light");
        tap(&mut rt, "row-7");
        assert_eq!(text_of(&rt, "c1-intent"), "Clicked holiday.jpg");
    }

    struct RenameOnPress;

    impl PointerController for RenameOnPress {
        fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
            if input.phase == PointerPhase::Down
                && let Some(i) = input.hit.index
            {
                let id = cx.source.id_at(i);
                cx.apply(|sel, src| sel.select_only(src, i));
                cx.emit(Effect::Intent(Intent::BeginRename { id }));
            }
            Flow::Handled
        }
    }

    /// `BeginRename` opens the rename slot on its row; Enter renames, Escape cancels.
    #[test]
    fn a_rename_intent_opens_the_rename_slot() {
        let mut rt = mount("c1-ten-rows-light");
        rt.update(|g, _| g.c1.coll.controllers.pointer = Some(Box::new(RenameOnPress)));
        tap(&mut rt, "row-4");
        assert_eq!(text_of(&rt, "c1-intent"), "Renaming Notes.md");
        let field = element(&rt, "c1-rename").expect("the rename slot is open");
        assert!(field.focused, "and has focus");
        key(&mut rt, Key::End, Mods::default());
        rt.key(KeyEvent {
            key: Key::Text("x".into()),
            mods: Mods::default(),
        });
        run(&mut rt, 0.1);
        key(&mut rt, Key::Enter, Mods::default());
        assert!(element(&rt, "c1-rename").is_none(), "Enter closes it");
        assert_eq!(text_of(&rt, "c1-intent"), "Renamed to Notes.mdx");
        let d = &rt.state.c1;
        assert_eq!(d.listing.entries()[3].name, "Notes.mdx");

        tap(&mut rt, "row-5");
        assert!(element(&rt, "c1-rename").is_some());
        key(&mut rt, Key::Escape, Mods::default());
        assert!(element(&rt, "c1-rename").is_none(), "Escape closes it");
        assert_eq!(rt.state.c1.listing.entries()[4].name, "budget.ods");
    }

    #[test]
    #[cfg_attr(debug_assertions, ignore = "perf: run with --profile perf")]
    fn scrolling_a_hundred_thousand_entries_stays_under_the_commit_budget() {
        let script = support::scripts_dir("c1").join("scroll-100k.json");
        let spec: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&script).expect("the script"))
                .expect("JSON");
        let events = serde_json::json!({ "events": spec["events"] });
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c1");
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let file = dir.join("scroll-100k-events.json");
        std::fs::write(&file, events.to_string()).expect("the events");

        let mut cmd = support::gallery();
        cmd.env("RUNE_COMMIT_BUDGET_MS", "8")
            .env(WARMUP, "1")
            .arg("--scene")
            .arg("c1-hundred-k-light")
            .arg("--script")
            .arg(&file)
            .arg("--tree")
            .arg(dir.join("scroll-100k-tree.json"));
        let stderr = match support::run(cmd) {
            Ok(stderr) => stderr,
            Err(e) => panic!("{e}"),
        };
        let after = stderr
            .split_once(WARMED)
            .map(|(_, after)| after)
            .unwrap_or_else(|| panic!("the page never warmed up:\n{stderr}"));
        let over: Vec<&str> = after
            .lines()
            .filter(|l| l.starts_with("rune: commit took"))
            .collect();
        assert!(
            over.is_empty(),
            "commits over budget while scrolling:\n{}",
            over.join("\n")
        );
    }
}
