// SPDX-License-Identifier: MIT

//! C2d on C2's page: the six key behaviours where the Columns view now does what Strata's
//! GTK view does (slice decision 17, `docs/rune/slice/V8-read-path-parity.md`). Each test
//! names the GTK code it follows, where a script can't see the result: the cursor, the
//! anchor and what the view tells its host.

mod c2d {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::columns::keys;
    use strata_gallery::Gallery;
    use strata_gallery::scenes::{self, c2::id_in};

    const DT: f64 = 1.0 / 60.0;

    /// The three-columns scene: Home, Documents (open from Home, active, its cursor on
    /// Reports) and Reports (open from Documents, with no cursor).
    fn three_columns() -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == "c2-three-columns-light")
            .map(|(_, f)| f)
            .expect("the scene");
        let mut g = Gallery::default();
        f(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        settle(&mut rt, 40);
        rt
    }

    fn settle(rt: &mut Runtime<Gallery>, frames: usize) {
        for _ in 0..frames {
            rt.frame(DT);
        }
    }

    fn info(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
    }

    /// A press on column `depth`'s title: the column takes the keyboard.
    fn focus_column(rt: &mut Runtime<Gallery>, depth: usize) {
        let title = keys::title(&format!("c2-col-{}", depth + 1));
        let [x, y, w, h] = info(rt, &title).expect("the title").visible;
        let (x, y) = (x + w / 2.0, y + h / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_up(x, y);
        settle(rt, 3);
        assert_eq!(rt.state.c2.columns.active(), depth);
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        settle(rt, 2);
    }

    fn key(rt: &mut Runtime<Gallery>, k: Key) {
        press(rt, k, Mods::default());
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::default()
        }
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::default()
        }
    }

    fn selected(rt: &Runtime<Gallery>, depth: usize) -> Vec<u64> {
        let d = &rt.state.c2;
        let c = &d.columns.column(depth).expect("column").collection;
        let ids = c.selection.selected_ids(&d.listings.0[depth].source);
        ids.into_iter().map(|id| id.get()).collect()
    }

    fn cursor(rt: &Runtime<Gallery>, depth: usize) -> Option<u64> {
        let c = &rt
            .state
            .c2
            .columns
            .column(depth)
            .expect("column")
            .collection;
        c.selection.cursor().map(|id| id.get())
    }

    fn id(parent: &str, name: &str) -> u64 {
        id_in(parent, name).get()
    }

    fn path(rt: &Runtime<Gallery>) -> String {
        let names = rt.state.c2.listings.0.iter().map(|c| c.name.as_str());
        names.collect::<Vec<_>>().join("/")
    }

    #[test]
    fn return_on_the_folder_whose_column_is_open_enters_it_on_its_first_entry() {
        // browser/view-modes/columns. `Browser::activate_focused` ends in `focus_child`
        // (`crates/strata-fs/src/app/browser/activation.rs`): the open column takes the
        // keyboard with the cursor on its first entry when it has none.
        let mut rt = three_columns();
        focus_column(&mut rt, 1);
        assert_eq!(cursor(&rt, 1), Some(id("Documents", "Reports")));
        assert_eq!(cursor(&rt, 2), None);

        key(&mut rt, Key::Enter);

        assert_eq!(rt.state.c2.columns.active(), 2);
        let first = id("Reports", "report-q0001.pdf");
        assert_eq!(
            (cursor(&rt, 2), selected(&rt, 2)),
            (Some(first), vec![first])
        );
        assert_eq!(rt.state.c2.intent, "Keyboard in column 2");
        assert_eq!(
            path(&rt),
            "Home/Documents/Reports",
            "nothing opened or closed"
        );
        // The column keeps a cursor it already has.
        key(&mut rt, Key::Down);
        let second = cursor(&rt, 2);
        key(&mut rt, Key::Left);
        assert_eq!(rt.state.c2.intent, "Keyboard in column 1");
        key(&mut rt, Key::Enter);
        assert_eq!((rt.state.c2.columns.active(), cursor(&rt, 2)), (2, second));
    }

    #[test]
    fn an_arrow_at_the_lists_end_selects_the_cursors_entry_alone() {
        // browser/selection. `Browser::move_selection` re-places the cursor whether or not
        // it moved (`crates/strata-fs/src/app/navigation/cursor.rs`), which makes its entry
        // the selection.
        let mut rt = three_columns();
        focus_column(&mut rt, 0);
        let (first, last) = (id("Home", "Documents"), id("Home", "report-final.pdf"));

        // Several selected, the cursor on the last entry.
        key(&mut rt, Key::Home);
        press(&mut rt, Key::End, shift());
        assert_eq!((selected(&rt, 0).len(), cursor(&rt, 0)), (13, Some(last)));
        key(&mut rt, Key::Down);
        assert_eq!(selected(&rt, 0), [last]);

        // Several selected, the cursor on the first entry.
        press(&mut rt, Key::Home, shift());
        assert_eq!((selected(&rt, 0).len(), cursor(&rt, 0)), (13, Some(first)));
        key(&mut rt, Key::Up);
        assert_eq!(selected(&rt, 0), [first]);

        // Nothing selected, after Escape.
        key(&mut rt, Key::Escape);
        assert!(selected(&rt, 0).is_empty());
        key(&mut rt, Key::Up);
        assert_eq!(
            (selected(&rt, 0), cursor(&rt, 0)),
            (vec![first], Some(first))
        );
    }

    #[test]
    fn a_key_that_leaves_the_cursor_in_place_still_reports_the_cursor() {
        // browser/view-modes/columns. `Browser::page_along` and `move_selection` raise
        // `FocusChanged` for the unchanged position, which is what the GTK views mirror from
        // (`on_focus_changed` in `src/ui/browser/events/selection.rs`). The page's stand-in
        // editor closes when the view reports the cursor or the selection.
        for stays in [Key::Home, Key::PageUp, Key::Up] {
            let mut rt = three_columns();
            focus_column(&mut rt, 0);
            key(&mut rt, Key::Home);
            let first = id_in("Home", "Documents");
            assert_eq!(selected(&rt, 0), [first.get()]);
            rt.update(|g, _| g.c2.renaming = Some((0, first)));

            key(&mut rt, stays.clone());

            assert_eq!(cursor(&rt, 0), Some(first.get()), "{stays:?}");
            assert_eq!(rt.state.c2.renaming, None, "{stays:?} reported nothing");
        }
    }

    #[test]
    fn ctrl_a_takes_the_cursor_to_the_last_entry_and_the_anchor_stays() {
        // browser/selection. In Columns Ctrl+A selects through GTK's selection model, and
        // `SelectionSync` reports the newest selected row as the cursor
        // (`src/ui/browser/columns/append/models.rs`); `set_selection` keeps the anchor.
        let mut rt = three_columns();
        focus_column(&mut rt, 0);
        key(&mut rt, Key::Home);
        key(&mut rt, Key::Down);
        let anchor = id("Home", "Pictures");
        assert_eq!(cursor(&rt, 0), Some(anchor));

        press(&mut rt, Key::Command('a'), ctrl());

        assert_eq!(selected(&rt, 0).len(), 13);
        assert_eq!(cursor(&rt, 0), Some(id("Home", "report-final.pdf")));
        // A plain arrow moves from the last entry, and a Shift arrow ranges from the old
        // anchor to the entry before the last.
        press(&mut rt, Key::Up, shift());
        let ranged = selected(&rt, 0);
        assert_eq!(ranged.len(), 11, "{ranged:?}");
        assert!(ranged.contains(&anchor) && !ranged.contains(&id("Home", "Documents")));
        assert_eq!(cursor(&rt, 0), Some(id("Home", "budget.ods")));
    }

    #[test]
    fn left_and_right_between_open_columns_tell_the_host_which_column_has_the_keyboard() {
        // browser/view-modes/columns. `Browser::focus_parent` and `focus_child` raise
        // `FocusChanged` for the column the keyboard lands in, so the GTK views mirror its
        // cursor; the view has to say so, since no cursor moved in the column it left.
        let mut rt = three_columns();
        focus_column(&mut rt, 1);

        key(&mut rt, Key::Left);

        assert_eq!(rt.state.c2.columns.active(), 0);
        assert_eq!(rt.state.c2.intent, "Keyboard in column 0");

        key(&mut rt, Key::Right);

        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(rt.state.c2.intent, "Keyboard in column 1");
        assert_eq!(path(&rt), "Home/Documents/Reports");
        // A press that makes a column active is the pointer's, and is not reported.
        focus_column(&mut rt, 0);
        assert_eq!(rt.state.c2.intent, "Keyboard in column 1");
    }

    #[test]
    fn right_opens_the_cursors_folder_when_the_open_column_is_another_folders() {
        // browser/view-modes/columns. `Browser::enter_focused_directory`
        // (`crates/strata-fs/src/app/browser/activation.rs`): a folder that is not the open
        // child is descended into, in the open column's place; a file enters the column
        // that is open.
        let mut rt = three_columns();
        focus_column(&mut rt, 0);
        // The cursor leaves Documents for Pictures; this page mirrors nothing, so
        // Documents' column stays open.
        key(&mut rt, Key::Down);
        assert_eq!(cursor(&rt, 0), Some(id("Home", "Pictures")));
        assert_eq!(path(&rt), "Home/Documents/Reports");

        key(&mut rt, Key::Right);

        assert_eq!(rt.state.c2.intent, "Entered Pictures");
        assert_eq!(path(&rt), "Home/Pictures");

        // On a file, Right goes into whatever column is open.
        key(&mut rt, Key::Left);
        key(&mut rt, Key::End);
        assert_eq!(cursor(&rt, 0), Some(id("Home", "report-final.pdf")));
        key(&mut rt, Key::Right);
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(rt.state.c2.intent, "Keyboard in column 1");
        assert_eq!(path(&rt), "Home/Pictures");
    }
}
