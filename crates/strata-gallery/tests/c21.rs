//! C21's page in process: what scripts can't reach. The scripts in `tests/scripts/c21/`
//! check the rest. Each test names the bullets it covers.

mod c21 {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
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
        run(&mut rt, 1.0);
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
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
    }

    fn press_with(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent {
            key: key.clone(),
            mods,
        });
        rt.frame(FRAME);
        rt.key_up(KeyEvent { key, mods });
        run(rt, 0.1);
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key) {
        press_with(rt, key, Mods::default());
    }

    /// A right-click at window point (`x`, `y`).
    fn right_click(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.secondary_down(x, y);
        rt.frame(FRAME);
        rt.secondary_up(x, y);
        run(rt, 0.2);
    }

    fn status(rt: &Runtime<Gallery>) -> String {
        rt.state.c21.status.clone()
    }

    /// Tab past the search field into the list.
    fn tab_into_the_list(rt: &mut Runtime<Gallery>) {
        for _ in 0..2 {
            press(rt, Key::Tab);
        }
    }

    // browser/context-menu: "A keyboard-opened item menu anchors to the entry, not the
    // pointer". The cursor's entry scrolled out of view is revealed first, and the menu opens
    // at it (where, the unit tests and the keyboard-anchor goldens show).
    #[test]
    fn the_menu_key_reveals_a_cursor_scrolled_out_of_view() {
        let mut rt = mount("c21-start-light");
        tab_into_the_list(&mut rt);
        press(&mut rt, Key::End);
        rt.update(|_, tx| {
            tx.scroll_to_instant(
                strata_components::collection::element::viewport_path(
                    strata_gallery::scenes::c21::SCOPE,
                ),
                0.0,
                0.0,
            )
        });
        run(&mut rt, 0.2);
        let row = element(&rt, "row-40");
        assert!(
            row.is_none_or(|r| r.visible[3] <= 0.0),
            "the cursor's row is out of view"
        );
        press(&mut rt, Key::ContextMenu);
        run(&mut rt, 0.5);
        assert_eq!(status(&rt), "Menu 1 on scan-36.pdf");
        let row = element(&rt, "row-40").expect("revealed");
        assert!(row.visible[3] > 0.0, "the cursor's row is back in view");
        let vp = rt.state.c21.coll.viewport();
        let at = rt.state.c21.menu.at().expect("open");
        assert!(
            at.y >= vp.y && at.y <= vp.y + vp.h,
            "the menu opens where the row is"
        );
    }

    // browser/context-menu: "A menu opens with keyboard focus on its first visible action".
    // After a right-click the first action has the keyboard but shows no highlight until a key
    // asks for it, as NSMenu (rune-kit's model): the first Down shows it there, the next moves.
    // Enter at once runs it.
    #[test]
    fn a_right_click_menu_has_the_keyboard_on_its_first_action() {
        let mut rt = mount("c21-nothing-light");
        let [x, y, w, h] = element(&rt, "row-5").unwrap().rect;
        rt.secondary_down(x + w / 2.0, y + h / 2.0);
        rt.frame(FRAME);
        rt.secondary_up(x + w / 2.0, y + h / 2.0);
        run(&mut rt, 0.2);
        assert_eq!(focused(&rt).as_deref(), Some("c21-menu-item3"));
        press(&mut rt, Key::Down);
        assert_eq!(
            focused(&rt).as_deref(),
            Some("c21-menu-item3"),
            "shown, not moved"
        );
        press(&mut rt, Key::Down);
        assert_eq!(focused(&rt).as_deref(), Some("c21-menu-item4"));
    }

    /// F2 on Notes.md, then a right-click on C5's field: its field menu is open.
    fn rename_menu_open() -> Runtime<Gallery> {
        let mut rt = mount("c21-start-light");
        tab_into_the_list(&mut rt);
        press(&mut rt, Key::Function(2));
        assert!(rt.state.c21.rename.is_open(), "F2 opened C5's editor");
        let [x, y, w, h] = element(&rt, "rename-field").expect("the field").rect;
        right_click(&mut rt, x + w / 2.0, y + h / 2.0);
        assert!(
            element(&rt, "c21-field-menu-menu").is_some(),
            "the field menu"
        );
        assert_eq!(status(&rt), "Field menu on the rename of Notes.md");
        rt
    }

    // C5's editor sends `RenameIntent::FieldMenu`; the host opens C21's field menu and calls
    // `menu_opened`, so the field losing the keyboard to the menu doesn't end the edit, and
    // `menu_closed` from the menu's `on_close`.
    #[test]
    fn c5s_field_menu_holds_the_edit_open_and_gives_the_keyboard_back() {
        let mut rt = rename_menu_open();
        run(&mut rt, 0.3);
        assert!(
            rt.state.c21.rename.is_open(),
            "the menu taking the keyboard isn't a blur"
        );
        press(&mut rt, Key::Escape);
        assert!(element(&rt, "c21-field-menu-menu").is_none());
        assert!(
            rt.state.c21.rename.is_open(),
            "Escape closed the menu, not the edit"
        );
        assert_eq!(focused(&rt).as_deref(), Some("rename-field"));
        assert_eq!(status(&rt), "The rename's field menu closed");
    }

    // B1 with C5: the entry deleted while its field menu is open ends the edit, and the
    // page's poll (`FieldMenu::check`) closes the menu and tells `menu_closed`, so it can't
    // come back over the next rename.
    #[test]
    fn an_entry_deleted_under_its_rename_field_menu_closes_the_menu() {
        use strata_components::collection::selection::{CursorGone, SyncRule};
        use strata_components::contract::EntryId;
        let mut rt = rename_menu_open();
        rt.update(|g, tx| {
            let d = &mut g.c21;
            d.listing.retain(|e| e.id != EntryId::new(5));
            d.coll.sync(
                strata_gallery::scenes::c21::SCOPE,
                &d.listing,
                SyncRule::complete(CursorGone::Neighbor),
                tx,
            );
        });
        run(&mut rt, 0.3);
        assert!(
            !rt.state.c21.rename.is_open(),
            "the edit ended with its entry"
        );
        assert!(!rt.state.c21.fields.is_open(), "and its field menu closed");
        assert_eq!(status(&rt), "The rename's field menu closed");
        // The next rename, the same field: no stale menu comes back.
        press(&mut rt, Key::Function(2));
        assert!(rt.state.c21.rename.is_open());
        assert!(element(&rt, "c21-field-menu-menu").is_none());
    }

    // browser/context-menu: "Right-clicking an entry anywhere in a long, scrolled list shows
    // its menu at the click." The script `scrolled-list` checks the target; this, the point.
    #[test]
    fn a_right_click_in_a_scrolled_list_opens_the_menu_at_the_click() {
        let mut rt = mount("c21-start-light");
        tab_into_the_list(&mut rt);
        press(&mut rt, Key::End);
        run(&mut rt, 0.5);
        let [x, y, w, h] = element(&rt, "row-40").expect("revealed").visible;
        let (px, py) = (x + w * 0.3, y + h / 2.0);
        right_click(&mut rt, px, py);
        assert_eq!(status(&rt), "Menu 1 on scan-36.pdf");
        let at = rt.state.c21.menu.at().expect("open");
        assert_eq!((at.x, at.y), (px, py));
    }

    // browser/context-menu: "In a small window, right-clicking an entry near the bottom edge
    // opens the menu at the click." Where it draws (flipped above, slid left) is the
    // `c21-edge` golden's.
    #[test]
    fn a_right_click_near_the_bottom_right_corner_opens_the_menu_at_the_click() {
        let mut rt = mount("c21-start-light");
        right_click(&mut rt, 900.0, 610.0);
        assert_eq!(status(&rt), "Menu 1 on scan-18.pdf");
        let at = rt.state.c21.menu.at().expect("open");
        assert_eq!((at.x, at.y), (900.0, 610.0));
        let menu = element(&rt, "c21-menu-menu").expect("drawn");
        let [mx, my, mw, mh] = menu.rect;
        assert!(
            mx >= 0.0 && my >= 0.0 && mx + mw <= 960.0 && my + mh <= 640.0,
            "inside"
        );
    }

    // browser/context-menu: "Screen readers get each row's label as its name and its shortcut
    // as its description." The shortcut is the item's keyboard-shortcut property (the golden
    // trees leave shortcuts out, as each platform spells them its way).
    #[test]
    fn a_rows_shortcut_is_its_keyboard_shortcut_property() {
        let rt = mount("c21-item-light");
        let tree = rt.accessibility_json();
        fn find<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
            if v["key"] == key {
                return Some(v);
            }
            v["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        let copy = find(&tree, "c21-menu-item12").expect("Copy");
        assert_eq!(copy["name"], "Copy");
        assert_eq!(copy["shortcut"], "Ctrl+C");
        assert!(copy.get("description").is_none(), "not the description");
    }
}
