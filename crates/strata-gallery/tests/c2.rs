//! C2's page: what the columns view does in process, beside its goldens and scripts. Each test
//! names the feature-map Behavior bullets it checks, where a script can't see them: scroll
//! offsets, widths, timing, modifiers held on a pointer, and paint.

mod c2 {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime};
    use serde_json::Value;
    use strata_components::columns::keys;
    use strata_components::contract::EntrySource;
    use strata_gallery::Gallery;
    use strata_gallery::scenes::{self, c2::id_in};

    const DT: f64 = 1.0 / 60.0;

    fn scene(name: &str) -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
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

    fn at(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        let [x, y, w, h] = info(rt, key)
            .unwrap_or_else(|| panic!("no element {key}"))
            .visible;
        (x + w / 2.0, y + h / 2.0)
    }

    fn click_at(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_up(x, y);
        rt.frame(DT);
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = at(rt, key);
        click_at(rt, x, y);
        settle(rt, 2);
    }

    /// A press on `key`'s name, near the start of its row.
    fn tap_name(rt: &mut Runtime<Gallery>, key: &str) {
        let [x, y, _, h] = info(rt, key).expect("row").visible;
        click_at(rt, x + 40.0, y + h / 2.0);
        settle(rt, 2);
    }

    fn press(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(DT);
        rt.frame(DT);
    }

    fn key(rt: &mut Runtime<Gallery>, k: Key) {
        press(rt, k, Mods::default());
    }

    /// A key press and nothing else: no frames.
    fn key_down(rt: &mut Runtime<Gallery>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::default()
        }
    }

    fn row(parent: &str, name: &str) -> String {
        format!("row-{}", id_in(parent, name).get())
    }

    fn path(rt: &Runtime<Gallery>) -> String {
        rt.state
            .c2
            .listings
            .0
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn selected(rt: &Runtime<Gallery>, depth: usize) -> Vec<u64> {
        let d = &rt.state.c2;
        let c = &d.columns.column(depth).expect("column").collection;
        c.selection
            .selected_ids(&d.listings.0[depth].source)
            .into_iter()
            .map(|id| id.get())
            .collect()
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

    /// The accessibility node keyed `key`.
    fn node(rt: &Runtime<Gallery>, key: &str) -> Option<Value> {
        fn find(v: &Value, key: &str) -> Option<Value> {
            if v["key"] == key {
                return Some(v.clone());
            }
            v["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        find(&rt.accessibility_json(), key)
    }

    fn focus_strip(rt: &mut Runtime<Gallery>) {
        // A press on the blank space after the last column focuses the strip and clears the
        // selection; a press on a column's title focuses it and keeps it.
        let (x, y) = at(rt, &keys::title("c2-col-1"));
        click_at(rt, x, y);
    }

    #[test]
    fn a_click_on_a_folder_opens_it_as_the_next_column() {
        // browser/view-modes/columns: With default click settings, a single click on a folder
        // opens it as the next column.
        let mut rt = scene("c2-home-light");
        tap_name(&mut rt, &row("Home", "Pictures"));
        assert_eq!(path(&rt), "Home/Pictures");
        assert_eq!(rt.state.c2.columns.active(), 1, "the new column is active");
        assert_eq!(cursor(&rt, 1), None, "with nothing selected in it");
    }

    #[test]
    fn right_and_left_move_the_cursor_between_columns_and_only_one_shows_it() {
        let mut rt = scene("c2-keyboard-light");
        focus_strip(&mut rt);
        assert_eq!(rt.state.c2.columns.active(), 0);
        // Right on Pictures, whose column is open: into it, keeping its multi-selection.
        key(&mut rt, Key::Right);
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(selected(&rt, 1).len(), 4, "its multi-selection stays");
        // Down moves Pictures' cursor, not Home's.
        let home = cursor(&rt, 0);
        key(&mut rt, Key::Down);
        assert_eq!(cursor(&rt, 0), home);
        // Left returns to Home, whose cursor stays on Pictures.
        key(&mut rt, Key::Left);
        assert_eq!(rt.state.c2.columns.active(), 0);
        assert_eq!(cursor(&rt, 0), Some(id("Home", "Pictures")));
        let strip = node(&rt, "c2").expect("strip");
        assert_eq!(strip["role"], "group");
        let pictures = format!("row-{}", id("Home", "Pictures"));
        assert_eq!(strip["activeDescendant"], pictures.as_str(), "{strip}");
    }

    #[test]
    fn the_close_button_closes_its_column_and_it_narrows_away() {
        let mut rt = scene("c2-three-columns-light");
        assert_eq!(path(&rt), "Home/Documents/Reports");
        tap(&mut rt, &keys::close("c2-col-2"));
        assert_eq!(path(&rt), "Home");
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.exiting && e.key.as_deref() == Some("c2-slot-2")),
            "it leaves on a transition"
        );
        settle(&mut rt, 40);
        assert!(
            !rt.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("c2-slot-2")),
            "then it's gone"
        );
    }

    #[test]
    fn a_click_on_a_folder_whose_column_is_open_focuses_and_reveals_that_column() {
        // browser/view-modes/columns: Clicking a folder whose child column is already open
        // focuses and reveals that column instead of closing it.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::title("c2-col-1"));
        assert_eq!(rt.state.c2.columns.active(), 0);
        // On its icon: a click on the name of the folder selected alone renames it (below).
        let [x, y, _, h] = info(&rt, &row("Home", "Documents")).expect("row").visible;
        click_at(&mut rt, x + 18.0, y + h / 2.0);
        settle(&mut rt, 2);
        assert_eq!(path(&rt), "Home/Documents/Reports", "nothing closes");
        assert_eq!(
            rt.state.c2.columns.active(),
            1,
            "Documents' column is active"
        );
        assert_ne!(
            rt.state.c2.intent, "Opened Documents",
            "the host isn't asked to open it"
        );
    }

    #[test]
    fn a_slow_second_click_on_an_open_folders_name_still_renames_it() {
        // browser/view-modes/columns: A slow second click on the name of a selected, open
        // folder still starts a rename.
        let mut rt = scene("c2-three-columns-light");
        tap_name(&mut rt, &row("Home", "Documents"));
        settle(&mut rt, 30);
        assert_eq!(rt.state.c2.intent, "Renaming Documents");
        assert_eq!(path(&rt), "Home/Documents/Reports");
    }

    #[test]
    fn escape_clears_the_active_column_then_closes_the_deepest() {
        // browser/selection: In Columns, Escape clears only the active column; parent
        // selections and open columns stay. And: Outside 10xer mode, in Columns, Escape with
        // nothing selected in the active column closes the deepest column.
        let mut rt = scene("c2-three-columns-light");
        tap_name(&mut rt, &row("Documents", "cv.pdf"));
        // The click on a file closes the deeper column (the page's stand-in): reopen Reports.
        tap_name(&mut rt, &row("Documents", "Reports"));
        tap_name(&mut rt, &row("Reports", "report-q0002.pdf"));
        assert_eq!(rt.state.c2.columns.active(), 2);
        key(&mut rt, Key::Escape);
        assert!(selected(&rt, 2).is_empty());
        assert_eq!(
            selected(&rt, 1),
            [id("Documents", "Reports")],
            "the parent keeps its own"
        );
        assert_eq!(path(&rt), "Home/Documents/Reports");
        key(&mut rt, Key::Escape);
        assert_eq!(
            path(&rt),
            "Home/Documents",
            "nothing selected: the deepest closes"
        );
    }

    #[test]
    fn a_click_on_empty_space_in_an_inactive_column_closes_deeper_and_picks_its_first_entry() {
        // browser/view-modes/columns: Clicking empty space in an inactive column closes deeper
        // columns, focuses and reveals that column, and selects its first visible entry.
        let mut rt = scene("c2-three-columns-light");
        let [x, y, w, h] = info(&rt, "c2-slot-1").expect("Home").visible;
        click_at(&mut rt, x + w / 2.0, y + h - 20.0);
        settle(&mut rt, 2);
        assert_eq!(path(&rt), "Home");
        assert_eq!(rt.state.c2.columns.active(), 0);
        assert_eq!(selected(&rt, 0), [id("Home", "Documents")]);
    }

    #[test]
    fn a_click_on_empty_space_in_the_active_column_clears_every_column() {
        // browser/view-modes/columns: Clicking empty space in the active column clears the
        // selection in every column and closes deeper columns.
        let mut rt = scene("c2-three-columns-light");
        assert!(!selected(&rt, 0).is_empty());
        // Below Documents' 27 rows there is no blank space at this size: use Reports.
        tap_name(&mut rt, &row("Reports", "report-q0001.pdf"));
        let [x, y, w, h] = info(&rt, "c2-slot-3").expect("Reports").visible;
        click_at(&mut rt, x + w / 2.0, y + h - 20.0);
        settle(&mut rt, 2);
        for depth in 0..3 {
            assert!(selected(&rt, depth).is_empty(), "column {depth}");
        }
        assert_eq!(
            path(&rt),
            "Home/Documents/Reports",
            "no deeper column to close"
        );
    }

    #[test]
    fn a_click_on_the_blank_strip_clears_the_selection() {
        // browser/view-modes/columns: Clicking the blank strip right of the last column clears
        // the selection.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::blank("c2"));
        for depth in 0..3 {
            assert!(selected(&rt, depth).is_empty(), "column {depth}");
        }
        assert_eq!(path(&rt), "Home/Documents/Reports");
    }

    #[test]
    fn a_click_on_a_title_focuses_its_column_keeping_its_selection_and_deeper_columns() {
        // browser/view-modes/columns: Clicking a column's title focuses that column, keeps its
        // selection, and leaves deeper columns open. And: Clicking a column heading focuses
        // that column without changing its selection or closing its child column.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::title("c2-col-1"));
        assert_eq!(rt.state.c2.columns.active(), 0);
        assert_eq!(selected(&rt, 0), [id("Home", "Documents")]);
        assert_eq!(path(&rt), "Home/Documents/Reports");
        // The header actions move with the target: Home has no close button.
        assert!(info(&rt, &keys::close("c2-col-1")).is_none());
        assert!(info(&rt, "c2-refresh-0").is_some());
        assert!(info(&rt, &keys::close("c2-col-2")).is_none());
    }

    #[test]
    fn a_secondary_press_on_a_header_opens_its_folder_menu_and_not_on_its_buttons() {
        // browser/view-modes/columns: Right-clicking a column's title or empty header space
        // opens that column's folder menu at the click; right-clicking a header button opens
        // no folder menu.
        let mut rt = scene("c2-three-columns-light");
        let (x, y) = at(&rt, &keys::title("c2-col-3"));
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.intent, "Folder menu for Reports");
        assert_eq!(rt.state.c2.columns.active(), 2);
        rt.state.c2.intent.clear();
        let (x, y) = at(&rt, "c2-refresh-2");
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.intent, "", "the button keeps its own");
    }

    #[test]
    fn a_loading_column_shows_a_spinner_until_it_is_listed() {
        // browser/view-modes/columns: The header spinner shows only while that column is
        // loading and is hidden when idle.
        let mut rt = scene("c2-home-light");
        tap_name(&mut rt, &row("Home", "Slow"));
        assert!(info(&rt, &keys::spinner("c2-col-2")).is_some());
        settle(&mut rt, 130);
        assert!(info(&rt, &keys::spinner("c2-col-2")).is_none());
        assert!(info(&rt, &row("Slow", "late-0001.txt")).is_some());
    }

    #[test]
    fn ctrl_a_page_and_home_act_in_the_active_column_only() {
        // browser/selection: Ctrl+A selects every entry in the focused column, not the deepest
        // open column. browser/scrolling: Page Up and Page Down move the focus and selection
        // by one page in the focused listing; in Columns only the focused column moves.
        // browser/view-modes/columns: In a column with no keyboard cursor yet, Home or Ctrl+Up
        // lands on the first entry and End or Ctrl+Down on the last.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::title("c2-col-1"));
        press(&mut rt, Key::Text("a".into()), ctrl());
        assert_eq!(selected(&rt, 0).len(), 13, "Home: all of it");
        assert_eq!(
            selected(&rt, 1),
            [id("Documents", "Reports")],
            "Documents untouched"
        );
        assert!(selected(&rt, 2).is_empty(), "Reports untouched");
        // Reports has no cursor: End lands on its last entry.
        tap(&mut rt, &keys::title("c2-col-3"));
        assert_eq!(cursor(&rt, 2), None);
        key(&mut rt, Key::End);
        assert_eq!(cursor(&rt, 2), Some(id("Reports", "report-q0008.pdf")));
        key(&mut rt, Key::Home);
        assert_eq!(cursor(&rt, 2), Some(id("Reports", "report-q0001.pdf")));
        // Page Down in Documents moves Documents' cursor only.
        tap(&mut rt, &keys::title("c2-col-2"));
        let before = cursor(&rt, 0);
        key(&mut rt, Key::PageDown);
        assert_ne!(cursor(&rt, 1), Some(id("Documents", "Reports")));
        assert_eq!(cursor(&rt, 0), before);
        assert_eq!(cursor(&rt, 2), Some(id("Reports", "report-q0001.pdf")));
    }

    fn strip_offset(rt: &Runtime<Gallery>) -> f32 {
        rt.state.c2.columns.strip_offset()
    }

    fn listing_offset(rt: &Runtime<Gallery>, depth: usize) -> f32 {
        rt.state
            .c2
            .columns
            .column(depth)
            .expect("column")
            .collection
            .scroll_offset()
    }

    #[test]
    fn a_double_click_on_a_closed_folder_ends_as_a_single_click() {
        // browser/view-modes/columns: Double-clicking a closed folder ends as a single click
        // does: its column opens, focused and scrolled into view, with nothing selected in it.
        // And: Pressing a folder does not shift the strip and shift it back before its column
        // takes focus.
        let mut rt = scene("c2-overflow-light");
        let before = strip_offset(&rt);
        assert!(before > 0.0, "scrolled to the active column");
        // level-5's column is last; its folder level-6 is closed.
        let [rx, ry, _, rh] = info(&rt, &row("level-5", "level-6")).expect("row").visible;
        let (x, y) = (rx + 18.0, ry + rh / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        assert_eq!(strip_offset(&rt), before, "the press moves no strip");
        rt.pointer_up(x, y);
        rt.frame(DT);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_up(x, y);
        settle(&mut rt, 40);
        assert!(path(&rt).ends_with("level-5/level-6"), "{}", path(&rt));
        let depth = rt.state.c2.listings.0.len() - 1;
        assert_eq!(rt.state.c2.columns.active(), depth, "focused");
        assert_eq!(cursor(&rt, depth), None, "nothing selected in it");
        assert!(strip_offset(&rt) > before, "scrolled into view");
    }

    #[test]
    fn with_double_click_folders_a_double_click_opens_it_from_a_clipped_parent() {
        // browser/view-modes/columns: The same holds when the first press scrolls the strip,
        // with double-click folders, and from a partly clipped parent column.
        let mut rt = scene("c2-three-columns-light");
        rt.state.c2.folder_clicks = strata_components::collection::pointer::ClickCount::Two;
        // A strip 870 wide, scrolled 30 points: Home is clipped by 30.
        rt.state.c2.width = Some(870.0);
        settle(&mut rt, 2);
        rt.update(|_, tx| tx.scroll_to_x(keys::strip("c2"), 30.0));
        settle(&mut rt, 2);
        assert!(
            strip_offset(&rt) > 20.0,
            "Home is clipped: {}",
            strip_offset(&rt)
        );
        let [rx, ry, _, rh] = info(&rt, &row("Home", "Pictures")).expect("row").visible;
        let (x, y) = (rx + 30.0, ry + rh / 2.0);
        click_at(&mut rt, x, y);
        assert_eq!(
            path(&rt),
            "Home/Documents/Reports",
            "one click opens no folder"
        );
        click_at(&mut rt, x, y);
        settle(&mut rt, 40);
        assert_eq!(path(&rt), "Home/Pictures");
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(cursor(&rt, 1), None);
    }

    #[test]
    fn a_plain_click_on_a_folder_in_a_multi_selection_collapses_it_and_opens_it() {
        // browser/view-modes/columns: With folders opening on one click, a plain click on a
        // folder inside a multi-selection collapses the selection to it and opens its column.
        let mut rt = scene("c2-home-light");
        tap_name(&mut rt, &row("Home", "notes.txt"));
        rt.physical_input(PhysicalInput::Modifiers(ctrl()));
        tap_name(&mut rt, &row("Home", "Music"));
        tap_name(&mut rt, &row("Home", "Projects"));
        rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
        assert_eq!(selected(&rt, 0).len(), 3);
        assert_eq!(path(&rt), "Home");
        let [x, y, _, h] = info(&rt, &row("Home", "Music")).expect("row").visible;
        click_at(&mut rt, x + 18.0, y + h / 2.0);
        settle(&mut rt, 2);
        assert_eq!(selected(&rt, 0), [id("Home", "Music")]);
        assert_eq!(path(&rt), "Home/Music");
    }

    #[test]
    fn a_file_drag_or_a_modified_click_in_a_parent_leaves_the_deeper_columns_open() {
        // browser/view-modes/columns: A press on a file that turns into a drag leaves the
        // deeper columns open, so the file can be dragged into the open column. And: Ctrl+click
        // and Shift+click on a file leave the deeper columns open. (The plain click that closes
        // them is Strata's policy, which the page stands in for.)
        let mut rt = scene("c2-three-columns-light");
        let [x, y, _, h] = info(&rt, &row("Home", "notes.txt")).expect("row").visible;
        let (x, y) = (x + 40.0, y + h / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        for k in 1..=6 {
            rt.pointer_move(x + 8.0 * k as f32, y);
            rt.frame(DT);
        }
        rt.pointer_up(x + 48.0, y);
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.intent, "Dragging 1 from column 0");
        assert_eq!(path(&rt), "Home/Documents/Reports");
        for mods in [
            ctrl(),
            Mods {
                shift: true,
                ..Mods::default()
            },
        ] {
            rt.physical_input(PhysicalInput::Modifiers(mods));
            tap_name(&mut rt, &row("Home", "todo.md"));
            rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
            assert_eq!(path(&rt), "Home/Documents/Reports", "{mods:?}");
        }
        // The page's stand-in for Strata: a plain click closes them.
        tap_name(&mut rt, &row("Home", "photo.jpg"));
        assert_eq!(path(&rt), "Home");
    }

    #[test]
    fn a_press_that_moves_less_than_the_threshold_starts_no_drag() {
        // browser/view-modes/columns: Pressing a row and moving less than GTK's drag threshold
        // starts no file drag.
        let mut rt = scene("c2-three-columns-light");
        let (x, y) = at(&rt, &row("Documents", "cv.pdf"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_move(x + 5.0, y + 3.0);
        rt.frame(DT);
        rt.pointer_up(x + 5.0, y + 3.0);
        settle(&mut rt, 2);
        assert!(
            !rt.state.c2.intent.starts_with("Dragging"),
            "{}",
            rt.state.c2.intent
        );
    }

    #[test]
    fn with_animations_off_a_closed_column_goes_at_once() {
        // browser/view-modes/columns: With reduced motion or animations off, a closing column
        // is removed at once.
        let mut rt = scene("c2-three-columns-light");
        rt.state.look.animations = false;
        settle(&mut rt, 2);
        tap(&mut rt, &keys::close("c2-col-2"));
        assert!(
            !rt.elements()
                .iter()
                .any(|e| e.key.as_deref() == Some("c2-slot-2")),
            "gone with no exit"
        );
    }

    #[test]
    fn a_closing_column_takes_no_presses_while_it_leaves() {
        // browser/view-modes/columns: A closing column takes no clicks, hovers, or drops while
        // it leaves.
        let mut rt = scene("c2-three-columns-light");
        let (x, y) = at(&rt, &row("Reports", "report-q0003.pdf"));
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.listings.0.truncate(2);
            g.c2.columns.truncate(2, &look, tx);
        });
        rt.frame(DT);
        rt.state.c2.intent.clear();
        click_at(&mut rt, x, y);
        assert!(
            !rt.state.c2.intent.contains("report-q0003"),
            "{}",
            rt.state.c2.intent
        );
    }

    #[test]
    fn a_rebuilt_strip_replaces_its_columns_with_no_exit() {
        // browser/view-modes/columns: Opening another location, rebuilding the strip, or
        // switching away from Columns replaces the columns with no exit animation.
        let mut rt = scene("c2-three-columns-light");
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            d.listings.0.truncate(1);
            d.columns.replace(&d.listings, 0, &|_| None, &look, tx);
        });
        rt.frame(DT);
        assert!(
            !rt.elements().iter().any(|e| e.exiting),
            "nothing leaves on a transition"
        );
        assert_eq!(path(&rt), "Home");
    }

    #[test]
    fn a_keyboard_move_reveals_a_clipped_column_and_leaves_a_shown_one() {
        // browser/view-modes/columns: A keyboard move reveals a column clipped by up to 4 px
        // fully and leaves an already visible column in place.
        let mut rt = scene("c2-overflow-light");
        let end = strip_offset(&rt);
        // Scroll the strip 3 points back: the active (last) column is clipped by 3 points.
        rt.update(|_, tx| tx.scroll_to_x(keys::strip("c2"), end - 3.0));
        settle(&mut rt, 2);
        assert!((strip_offset(&rt) - (end - 3.0)).abs() < 0.5);
        tap(&mut rt, &keys::title("c2-col-7"));
        assert!(
            (strip_offset(&rt) - (end - 3.0)).abs() < 0.5,
            "a press reveals nothing"
        );
        key(&mut rt, Key::Down);
        settle(&mut rt, 30);
        assert!((strip_offset(&rt) - end).abs() < 0.5, "revealed fully");
        key(&mut rt, Key::Down);
        settle(&mut rt, 30);
        assert!(
            (strip_offset(&rt) - end).abs() < 0.5,
            "a shown column stays"
        );
    }

    #[test]
    fn a_column_wider_than_the_strip_stays_while_it_fills_it() {
        // browser/view-modes/columns: A column wider than the viewport that already fills it
        // stays in place; a partly off-screen one scrolls to its left edge.
        let mut rt = scene("c2-wide-column-light");
        tap(&mut rt, &keys::title("c2-col-2"));
        settle(&mut rt, 30);
        // Music's column (1,100 wide) is active; the strip shows its left edge.
        let left = strip_offset(&rt);
        assert!(left > 0.0);
        rt.update(|_, tx| tx.scroll_to_x(keys::strip("c2"), left + 100.0));
        settle(&mut rt, 2);
        key(&mut rt, Key::Down);
        settle(&mut rt, 30);
        assert!(
            (strip_offset(&rt) - (left + 100.0)).abs() < 0.5,
            "it fills the strip"
        );
        // Scrolled so it shows only partly: back to its left edge.
        rt.update(|_, tx| tx.scroll_to_x(keys::strip("c2"), 0.0));
        settle(&mut rt, 2);
        key(&mut rt, Key::Down);
        settle(&mut rt, 30);
        assert!(
            (strip_offset(&rt) - left).abs() < 0.5,
            "{} vs {left}",
            strip_offset(&rt)
        );
    }

    #[test]
    fn back_to_an_ancestor_selects_the_folder_it_came_from_with_its_column_closed() {
        // browser/view-modes/columns: Back, Forward, or Alt+Up to an ancestor selects the folder
        // you came from, at any depth, with the cursor on it. And: The came-from folder's own
        // column stays closed.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::title("c2-col-3"));
        press(
            &mut rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
        assert_eq!(path(&rt), "Home/Documents");
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(selected(&rt, 1), [id("Documents", "Reports")]);
        assert_eq!(cursor(&rt, 1), Some(id("Documents", "Reports")));
    }

    #[test]
    fn left_into_a_column_with_no_cursor_puts_it_on_the_folder_it_has_open() {
        // browser/view-modes/columns: When Back leaves ancestor columns with no cursor, Left
        // into an ancestor column puts the cursor on the folder that column has open.
        let mut rt = scene("c2-three-columns-light");
        rt.update(|g, _| {
            let d = &mut g.c2;
            let src = &d.listings.0[1].source;
            let c = &mut d.columns.column_mut(1).unwrap().collection;
            c.selection
                .restore(&strata_components::collection::selection::Position::default());
            c.note_selection(src);
        });
        tap(&mut rt, &keys::title("c2-col-3"));
        key(&mut rt, Key::Down);
        key(&mut rt, Key::Left);
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(cursor(&rt, 1), Some(id("Documents", "Reports")));
    }

    #[test]
    fn left_into_a_loading_column_places_the_cursor_once_it_is_listed() {
        // browser/view-modes/columns: If that column is still loading, the cursor lands when it
        // finishes. If the folder is gone or hidden by then, nothing is selected.
        let mut rt = scene("c2-home-light");
        tap_name(&mut rt, &row("Home", "Slow"));
        // Slow is still listing; pretend a folder of it is open in a third column.
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            let mut listed = strata_gallery::scenes::c2::Listed {
                name: "late-0003.txt".into(),
                source: Default::default(),
                version: 1,
                due: None,
                from: Some(id_in("Slow", "late-0003.txt")),
            };
            listed.source = strata_components::contract::ListSource::new(Vec::new());
            d.listings.0.push(listed);
            d.columns.push(
                strata_components::columns::Opening::new(
                    &d.listings.0[2].source,
                    strata_components::collection::selection::Listing::Complete,
                ),
                strata_components::columns::Focus::Column,
                &look,
                tx,
            );
        });
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.columns.active(), 2);
        key(&mut rt, Key::Left);
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert_eq!(cursor(&rt, 1), None, "Slow is still listing");
        settle(&mut rt, 130);
        assert_eq!(cursor(&rt, 1), Some(id("Slow", "late-0003.txt")));
    }

    #[test]
    fn a_click_on_empty_space_keeps_the_columns_scroll() {
        // browser/view-modes/columns: Clicking empty space in a column keeps that column's
        // vertical scroll position instead of returning to the selected row. A column with blank
        // space below its rows can't be scrolled here, so the press reveals nothing in any
        // column: Documents, scrolled away from its selected row, stays where it is while a
        // click below Reports' rows takes effect.
        let mut rt = scene("c2-three-columns-light");
        rt.update(|g, tx| {
            let key = g.c2.columns.column_key(1).unwrap();
            tx.scroll_to_instant(
                strata_components::collection::element::viewport_path(&key),
                0.0,
                10_000.0,
            );
        });
        settle(&mut rt, 5);
        let documents = listing_offset(&rt, 1);
        assert!(documents > 100.0, "{documents}");
        assert_eq!(
            selected(&rt, 1),
            [id("Documents", "Reports")],
            "out of view"
        );
        let [x, y, w, h] = info(&rt, &row("Reports", "report-q0008.pdf"))
            .expect("the last row")
            .visible;
        // Blank space below Reports' last row.
        click_at(&mut rt, x + w / 2.0, y + h + 60.0);
        settle(&mut rt, 10);
        assert_eq!(
            selected(&rt, 2),
            [id("Reports", "report-q0001.pdf")],
            "the click took effect: Reports' first visible entry"
        );
        assert_eq!(
            listing_offset(&rt, 1),
            documents,
            "Documents keeps its scroll"
        );
        assert_eq!(listing_offset(&rt, 2), 0.0);
    }

    #[test]
    fn a_sideways_wheel_over_a_listing_scrolls_the_strip_and_a_vertical_one_the_listing() {
        // browser/view-modes/columns: Horizontal touchpad gestures over a listing scroll the
        // column strip. And the vertical half of: Shift+wheel over a listing scrolls the column
        // strip horizontally; vertical wheel ticks scroll only the listing. (Rune's headless
        // wheel doesn't turn Shift+wheel sideways: Rune draft U-C24-7.)
        let mut rt = scene("c2-overflow-light");
        let strip = strip_offset(&rt);
        let (x, y) = at(&rt, &row("level-4", "level-5"));
        rt.wheel_xy(x, y, 120.0, 0.0);
        settle(&mut rt, 20);
        assert!(strip_offset(&rt) < strip, "the strip moved back");
        let mut rt = scene("c2-hundred-k-light");
        let (strip, listing) = (strip_offset(&rt), listing_offset(&rt, 1));
        let [x, y, w, h] = info(&rt, "c2-slot-2").expect("Big").visible;
        rt.wheel(x + w / 2.0, y + h / 2.0, -200.0);
        settle(&mut rt, 20);
        assert_eq!(strip_offset(&rt), strip);
        assert!(listing_offset(&rt, 1) != listing);
    }

    /// Presses column `serial`'s edge `from` points right of its line and moves `dx` in four
    /// steps, holding the button: the caller releases (or presses Escape first).
    fn drag_edge(rt: &mut Runtime<Gallery>, serial: u64, dx: f32, from: f32) {
        let [x, y, _, h] = info(rt, &keys::edge(&format!("c2-col-{serial}")))
            .expect("edge")
            .visible;
        let (x, y) = (x + 6.0 + from, y + h / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        for k in 1..=4 {
            rt.pointer_move(x + dx * k as f32 / 4.0, y);
            rt.frame(DT);
        }
    }

    /// Column `depth`'s drawn width (its shell keyed by its slot, the depth plus one in these
    /// scenes).
    fn width(rt: &Runtime<Gallery>, depth: usize) -> f32 {
        let [_, _, w, _] = info(rt, &format!("c2-slot-{}", depth + 1))
            .expect("shell")
            .rect;
        w
    }

    #[test]
    fn dragging_an_edge_resizes_only_its_column_never_below_one_standard_column() {
        // browser/view-modes/columns: Dragging a column's right edge resizes only that column
        // while the pointer moves, never below 300 px. And: A resize ... is saved as
        // `browser_column_width`; new columns open at that width (the view hands the host the
        // width; saving it is the host's). The release spreading it is C2b's (tests/c2b.rs).
        let mut rt = scene("c2-three-columns-light");
        drag_edge(&mut rt, 1, -150.0, 0.0);
        assert_eq!(width(&rt, 0), 300.0, "never below one standard column");
        let (x, y) = rt.pointer().expect("pointer");
        rt.pointer_up(x, y);
        // Past the 400 ms in which a second press would fit the column (C2b).
        settle(&mut rt, 30);
        drag_edge(&mut rt, 1, 100.0, 0.0);
        assert_eq!(width(&rt, 0), 400.0, "it follows the pointer");
        assert_eq!(
            width(&rt, 1),
            300.0,
            "the others keep theirs while it moves"
        );
        let (x, y) = rt.pointer().expect("pointer");
        rt.pointer_up(x, y);
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.intent, "Width 400");
        // A new column opens at the width handed over.
        tap_name(&mut rt, &row("Reports", "report-q0001.pdf"));
        tap(&mut rt, &keys::title("c2-col-1"));
        tap_name(&mut rt, &row("Home", "Music"));
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 400.0, "{}", path(&rt));
    }

    #[test]
    fn a_press_either_side_of_the_line_resizes_the_left_column() {
        // browser/view-modes/columns: A press within 6 px either side of the line between two
        // columns resizes the left column, even when partly scrolled out of view.
        for from in [-5.0, 5.0] {
            let mut rt = scene("c2-three-columns-light");
            drag_edge(&mut rt, 1, 50.0, from);
            assert_eq!(width(&rt, 0), 350.0, "from {from}");
            assert_eq!(width(&rt, 1), 300.0);
        }
    }

    #[test]
    fn a_cancelled_edge_drag_puts_the_width_back() {
        // browser/view-modes/columns: A cancelled edge drag restores the column's previous
        // width.
        let mut rt = scene("c2-three-columns-light");
        drag_edge(&mut rt, 1, 120.0, 0.0);
        assert_eq!(width(&rt, 0), 420.0);
        key(&mut rt, Key::Escape);
        let (x, y) = rt.pointer().expect("pointer");
        rt.pointer_up(x, y);
        settle(&mut rt, 2);
        assert_eq!(width(&rt, 0), 300.0);
        assert_ne!(rt.state.c2.intent, "Width 420", "nothing handed over");
    }

    #[test]
    fn an_edge_drag_stops_at_the_area_and_the_cap_never_goes_below_a_standard_column() {
        // browser/view-modes/columns: Autofit and edge drags stop at the visible browser area
        // (the drag half; the area itself is the host's). And: The width cap never goes below
        // the 300 px standard column, scaled by text size.
        let mut rt = scene("c2-three-columns-light");
        rt.state.c2.columns.set_width_cap(Some(500.0));
        drag_edge(&mut rt, 1, 400.0, 0.0);
        assert_eq!(width(&rt, 0), 500.0, "stops at the area");
        let (x, y) = rt.pointer().expect("pointer");
        rt.pointer_up(x, y);
        let mut rt = scene("c2-three-columns-light");
        rt.state.c2.columns.set_width_cap(Some(200.0));
        drag_edge(&mut rt, 1, 100.0, 0.0);
        assert_eq!(width(&rt, 0), 300.0, "a cap below a standard column is one");
    }

    #[test]
    fn a_saved_width_wider_than_the_area_opens_columns_narrowed_to_it() {
        // browser/view-modes/columns: A saved width wider than the area, such as
        // `browser_column_width = 1729` in a 1200 px window, opens columns narrowed to the area.
        // The setting is unchanged.
        let mut rt = scene("c2-home-light");
        rt.state.c2.columns.set_default_width(1_729.0);
        rt.state.c2.columns.set_width_cap(Some(700.0));
        tap_name(&mut rt, &row("Home", "Music"));
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 700.0);
        rt.state.c2.columns.set_width_cap(None);
        tap(&mut rt, &keys::title("c2-col-1"));
        tap_name(&mut rt, &row("Home", "Projects"));
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 1_729.0, "the saved width is unchanged");
    }

    #[test]
    fn a_rest_on_a_column_edge_shows_the_width_caption() {
        // browser/view-modes/columns: Resting the pointer on a column edge for 400 ms shows a
        // "Column width" caption, which follows the edge during a drag.
        let mut rt = scene("c2-three-columns-light");
        let (x, y) = at(&rt, &keys::edge("c2-col-1"));
        rt.pointer_move(x, y);
        settle(&mut rt, 12);
        assert!(
            node(&rt, &keys::caption("c2-col-1")).is_none(),
            "not before 400 ms"
        );
        settle(&mut rt, 20);
        let caption = node(&rt, &keys::caption("c2-col-1")).expect("caption");
        assert_eq!(caption["name"], "Column width");
        let [before, ..] = info(&rt, &keys::caption("c2-col-1")).expect("caption").rect;
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_move(x + 60.0, y);
        settle(&mut rt, 2);
        let [after, ..] = info(&rt, &keys::caption("c2-col-1"))
            .expect("during the drag")
            .rect;
        assert_eq!(after - before, 60.0, "it rides the edge");
        rt.pointer_up(x + 60.0, y);
    }

    #[test]
    fn the_edge_of_the_column_beside_the_preview_lights_the_divider_line() {
        // browser/view-modes/columns: Hovering the edge of the column that meets the preview
        // also lights the preview divider line.
        let mut rt = scene("c2-preview-slot-light");
        rt.update(|g, _| {
            if let Some(d) = g.c2.drawer.as_mut() {
                d.open_at_once();
            }
        });
        settle(&mut rt, 10);
        // The last column active, so its edge meets the preview.
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.columns.set_active(2, &look, tx);
        });
        settle(&mut rt, 40);
        let lit = |rt: &Runtime<Gallery>| info(rt, "preview-divider-lit").map(|e| e.opacity);
        assert_eq!(lit(&rt), Some(0.0));
        let (x, y) = at(&rt, &keys::edge("c2-col-3"));
        rt.pointer_move(x - 3.0, y);
        settle(&mut rt, 60);
        assert_eq!(lit(&rt), Some(1.0));
        rt.pointer_move(x - 200.0, y);
        settle(&mut rt, 40);
        assert_eq!(lit(&rt), Some(0.0));
    }

    #[test]
    fn a_secondary_press_on_a_header_targets_its_column_for_the_keys() {
        // browser/view-modes/columns: Escape on a header-opened folder menu returns arrow-key
        // navigation to that column (the menu is C21's; the column it returns to is the view's).
        // And: While an item or background context menu is open in a column, that column keeps
        // the header actions even when the pointer moves over another column.
        let mut rt = scene("c2-three-columns-light");
        let (x, y) = at(&rt, &keys::title("c2-col-1"));
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        settle(&mut rt, 2);
        rt.state.c2.columns.hold_target(Some(0));
        // The pointer presses another column's title: the held one keeps the actions.
        let (x2, y2) = at(&rt, &keys::title("c2-col-3"));
        rt.pointer_move(x2, y2);
        settle(&mut rt, 2);
        assert!(info(&rt, "c2-refresh-0").is_some());
        rt.state.c2.columns.hold_target(None);
        // The menu closed (Escape, C21's): the arrows move Home's cursor.
        let before = cursor(&rt, 0);
        focus_strip(&mut rt);
        key(&mut rt, Key::Down);
        assert_ne!(cursor(&rt, 0), before);
    }

    // Column autoscroll during a drag (operations/drag-and-drop/column-autoscroll).

    fn drag_files(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.drag_files(vec!["/tmp/a.txt".into()], Some((x, y)));
        rt.frame(DT);
    }

    #[test]
    fn a_drag_near_the_strips_edge_scrolls_the_columns_toward_it_and_stops_when_it_leaves() {
        // operations/drag-and-drop/column-autoscroll: Holding a file drag within 44 px of the
        // column strip's left or right edge scrolls the columns toward that edge. And:
        // Autoscroll stops when the drag leaves the strip, drops, or ends, or the content
        // cannot scroll further.
        let mut rt = scene("c2-overflow-light");
        let start = strip_offset(&rt);
        let strip = rt.state.c2.columns.strip_rect();
        let y = strip.y + strip.h / 2.0;
        drag_files(&mut rt, strip.x + 10.0, y);
        settle(&mut rt, 20);
        let moved = strip_offset(&rt);
        assert!(moved < start - 20.0, "{moved} from {start}");
        // Away from the edge: it stops where it is.
        rt.drag_files_to(Some((strip.x + strip.w / 2.0, y)));
        settle(&mut rt, 3);
        let stopped = strip_offset(&rt);
        settle(&mut rt, 20);
        assert_eq!(strip_offset(&rt), stopped);
        // Back at the edge, then the drag goes: it stops.
        rt.drag_files_to(Some((strip.x + 10.0, y)));
        settle(&mut rt, 10);
        rt.drag_files_leave();
        settle(&mut rt, 2);
        let left = strip_offset(&rt);
        settle(&mut rt, 20);
        assert_eq!(strip_offset(&rt), left);
        // At the start it can't go further: it rests there.
        drag_files(&mut rt, strip.x + 10.0, y);
        settle(&mut rt, 200);
        assert!(strip_offset(&rt) < 1.0, "{}", strip_offset(&rt));
        rt.drag_files_leave();
        rt.frame(DT);
    }

    #[test]
    fn a_drag_near_a_listings_bottom_scrolls_that_listing() {
        // operations/drag-and-drop/column-autoscroll: Holding a file drag within 44 px of the
        // top or bottom edge of the listing under the pointer scrolls that listing.
        let mut rt = scene("c2-three-columns-light");
        let v = rt.state.c2.columns.column(1).unwrap().collection.viewport();
        drag_files(&mut rt, v.x + v.w / 2.0, v.y + v.h - 5.0);
        settle(&mut rt, 30);
        assert!(listing_offset(&rt, 1) > 20.0);
        assert_eq!(listing_offset(&rt, 0), 0.0);
        assert_eq!(listing_offset(&rt, 2), 0.0);
        rt.drag_files_leave();
        rt.frame(DT);
    }

    #[test]
    fn the_column_under_a_held_drag_is_washed_including_one_autoscroll_revealed() {
        // operations/drag-and-drop/column-autoscroll: The column under a held file drag shows
        // an accent wash and ring, including a column just revealed by autoscroll. And: The
        // highlight stays while autoscroll is at its limit and clears when the drag leaves the
        // column.
        let mut rt = scene("c2-overflow-light");
        let strip = rt.state.c2.columns.strip_rect();
        let y = strip.y + strip.h / 2.0;
        drag_files(&mut rt, strip.x + 10.0, y);
        settle(&mut rt, 2);
        let first = rt.state.c2.columns.drop_column();
        assert!(first.is_some());
        settle(&mut rt, 200);
        assert_eq!(
            rt.state.c2.columns.drop_column(),
            Some(0),
            "Home, revealed at the start"
        );
        assert!(
            info(&rt, &keys::wash("c2-col-1")).is_some(),
            "washed at the limit"
        );
        // Onto the blank strip... there is none here: leave the window instead.
        rt.drag_files_leave();
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.columns.drop_column(), None);
        assert!(info(&rt, &keys::wash("c2-col-1")).is_none());
    }

    #[test]
    fn a_drop_on_a_partly_shown_column_goes_there_and_the_strip_stays() {
        // operations/drag-and-drop/column-autoscroll: Dropping onto a partially visible column
        // moves the file there, and the strip does not scroll back to the source column.
        let mut rt = scene("c2-overflow-light");
        let start = strip_offset(&rt);
        let strip = rt.state.c2.columns.strip_rect();
        // The column clipped on the left (level-2's), 50 points in from the strip's edge.
        let at = (strip.x + 50.0, strip.y + strip.h - 60.0);
        rt.drop_files(vec!["/tmp/a.txt".into()], Some(at));
        settle(&mut rt, 20);
        assert_eq!(rt.state.c2.intent, "Dropped 1 into level-2");
        assert_eq!(strip_offset(&rt), start);
    }

    fn focused(rt: &Runtime<Gallery>) -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter(|e| e.focused)
            .filter_map(|e| e.key)
            .collect()
    }

    #[test]
    fn after_the_close_button_the_keys_still_reach_the_strip() {
        // browser/view-modes/columns: Each column after the first has a "Close this pane" X
        // button that closes it (and the keyboard goes on working: the strip keeps focus). And
        // bullet 58's half the view holds: the view moves no focus, so the strip keeps it.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::title("c2-col-2"));
        tap(&mut rt, &keys::close("c2-col-2"));
        assert_eq!(path(&rt), "Home");
        assert_eq!(focused(&rt), ["c2"], "the strip keeps focus");
        let before = cursor(&rt, 0);
        key(&mut rt, Key::Down);
        assert_ne!(cursor(&rt, 0), before, "Down moves Home's cursor");
        // A host's header action (the page's Refresh) leaves the keys with the strip too.
        tap(&mut rt, "c2-refresh-0");
        assert_eq!(rt.state.c2.intent, "Refreshed column 0");
        assert_eq!(focused(&rt), ["c2"]);
    }

    #[test]
    fn a_late_listing_in_a_column_left_behind_places_no_cursor_and_moves_no_strip() {
        // browser/view-modes/columns: If that column is still loading, the cursor lands when it
        // finishes (while the column holds the keyboard; one the person has left places none,
        // as Strata's restore gives up once focus leaves the list).
        let mut rt = scene("c2-home-light");
        rt.state.c2.width = Some(500.0);
        settle(&mut rt, 5);
        tap(&mut rt, &keys::title("c2-col-1"));
        rt.update(|g, _| {
            let d = &mut g.c2;
            let src = &d.listings.0[0].source;
            let c = &mut d.columns.column_mut(0).unwrap().collection;
            let i = src.index_of(id_in("Home", "Slow")).unwrap();
            let _ = c.selection.select_only(src, i);
            c.note_selection(src);
        });
        key(&mut rt, Key::Right);
        settle(&mut rt, 20);
        assert_eq!(path(&rt), "Home/Slow");
        assert_eq!(rt.state.c2.columns.active(), 1);
        key(&mut rt, Key::Left);
        settle(&mut rt, 20);
        assert_eq!(rt.state.c2.columns.active(), 0);
        let offset = strip_offset(&rt);
        settle(&mut rt, 130);
        assert_eq!(
            cursor(&rt, 1),
            None,
            "no cursor lands in the column left behind"
        );
        assert_eq!(
            strip_offset(&rt),
            offset,
            "the strip stays on the active column"
        );
    }

    #[test]
    fn a_keyboard_reveal_glides_the_strip() {
        // browser/view-modes/columns: A keyboard move reveals a column clipped by up to 4 px
        // fully (on the view's reveal glide, not by the row's own reveal jumping the strip).
        let mut rt = scene("c2-overflow-light");
        tap(&mut rt, &keys::title("c2-col-7"));
        let end = strip_offset(&rt);
        rt.update(|_, tx| tx.scroll_to_instant(keys::strip("c2"), 0.0, 0.0));
        settle(&mut rt, 5);
        assert_eq!(strip_offset(&rt), 0.0);
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
        rt.frame(DT);
        rt.frame(DT);
        let early = strip_offset(&rt);
        assert!(
            early > 0.0 && early < end - 50.0,
            "gliding: {early} of {end}"
        );
        settle(&mut rt, 40);
        assert!(
            (strip_offset(&rt) - end).abs() < 0.5,
            "{} vs {end}",
            strip_offset(&rt)
        );
    }

    /// Column `depth`'s laid-out width and how much of it shows (its shell keyed by its slot,
    /// the depth plus one in these scenes).
    fn shown(rt: &Runtime<Gallery>, depth: usize) -> (f32, f32) {
        assert!(
            rt.state.c2.columns.column(depth).is_some(),
            "column {depth}"
        );
        let i = info(rt, &format!("c2-slot-{}", depth + 1)).expect("slot");
        (i.rect[2], i.visible[2])
    }

    #[test]
    fn a_reveal_in_the_same_handler_as_a_narrower_strip_still_shows_the_column() {
        // browser/view-modes/columns: A keyboard move reveals a column ... fully (the reveal is
        // worked out again when the strip's width changes right after it).
        let mut rt = scene("c2-overflow-light");
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.columns.set_active(0, &look, tx);
        });
        settle(&mut rt, 60);
        assert!(strip_offset(&rt) < 0.5);
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.width = Some(700.0);
            g.c2.columns.set_active(6, &look, tx);
        });
        settle(&mut rt, 60);
        let (w, visible) = shown(&rt, 6);
        assert!((w - visible).abs() < 0.5, "level-5 shows {visible} of {w}");
    }

    #[test]
    fn a_drawer_opening_in_the_same_handler_as_a_close_keeps_the_active_column_whole() {
        // browser/view-modes/columns: A keyboard move reveals a column ... fully; here the host
        // closes the deepest column and opens an unreserved drawer in one handler.
        let mut rt = scene("c2-overflow-light");
        rt.update(|g, _| g.c2.drawer = Some(strata_components::preview_drawer::DrawerState::new()));
        settle(&mut rt, 20);
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.listings.0.truncate(6);
            g.c2.columns.truncate(6, &look, tx);
            g.c2.drawer.as_mut().unwrap().open(&look, tx);
        });
        settle(&mut rt, 90);
        assert_eq!(rt.state.c2.columns.active(), 5);
        let (w, visible) = shown(&rt, 5);
        assert!((w - visible).abs() < 0.5, "level-4 shows {visible} of {w}");
    }

    #[test]
    fn the_blank_strip_is_no_control_assistive_technology_hears() {
        // What assistive technology hears: the strip and its named lists, no unnamed button for
        // the blank space after the last column (a press there clears the selection).
        let rt = scene("c2-three-columns-light");
        assert!(
            info(&rt, &keys::blank("c2")).is_some(),
            "it is there to press"
        );
        assert!(
            node(&rt, &keys::blank("c2")).is_none(),
            "{:?}",
            node(&rt, &keys::blank("c2"))
        );
    }

    #[test]
    fn a_press_in_another_column_cancels_a_pending_slow_click_rename() {
        // browser/view-modes/columns: A slow second click on the name of a selected, open
        // folder still starts a rename (and a press in another column before it comes cancels
        // it, as Strata's cancel_click_rename does).
        let mut rt = scene("c2-three-columns-light");
        tap_name(&mut rt, &row("Home", "Documents"));
        tap(&mut rt, &keys::title("c2-col-3"));
        settle(&mut rt, 40);
        assert_ne!(rt.state.c2.intent, "Renaming Documents");
    }

    #[test]
    fn right_into_a_child_column_with_no_cursor_lands_on_its_first_entry() {
        // browser/view-modes/columns (and C1c's keyboard bullets in Columns): Right goes into the
        // open child column, with the cursor on its first entry when it has none.
        let mut rt = scene("c2-three-columns-light");
        tap(&mut rt, &keys::title("c2-col-2"));
        assert_eq!(cursor(&rt, 2), None);
        key(&mut rt, Key::Right);
        assert_eq!(rt.state.c2.columns.active(), 2);
        assert_eq!(cursor(&rt, 2), Some(id("Reports", "report-q0001.pdf")));
        assert_eq!(selected(&rt, 2), [id("Reports", "report-q0001.pdf")]);
    }

    #[test]
    fn back_to_an_ancestor_scrolls_the_came_from_folder_into_view() {
        // browser/view-modes/columns: Back, Forward, or Alt+Up to an ancestor selects the folder
        // you came from, at any depth, with the cursor on it (scrolled into view).
        let mut rt = scene("c2-hundred-k-light");
        let folder = id("Big", "entry-050001");
        let [x, y, _, h] = info(&rt, &format!("row-{folder}")).expect("row").visible;
        click_at(&mut rt, x + 18.0, y + h / 2.0);
        settle(&mut rt, 20);
        assert!(path(&rt).ends_with("Big/entry-050001"), "{}", path(&rt));
        rt.update(|g, tx| {
            let key = g.c2.columns.column_key(1).unwrap();
            tx.scroll_to_instant(
                strata_components::collection::element::viewport_path(&key),
                0.0,
                0.0,
            );
        });
        settle(&mut rt, 5);
        assert_eq!(listing_offset(&rt, 1), 0.0);
        press(
            &mut rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
        settle(&mut rt, 40);
        assert_eq!(cursor(&rt, 1), Some(folder));
        assert!(
            info(&rt, &format!("row-{folder}")).is_some(),
            "its row is on screen"
        );
        assert!(
            listing_offset(&rt, 1) > 1_000_000.0,
            "{}",
            listing_offset(&rt, 1)
        );
    }

    #[test]
    fn a_drop_clears_the_wash_even_when_the_binder_sends_no_leave() {
        // operations/drag-and-drop/column-autoscroll: The column under a held file drag shows an
        // accent wash (and a drop ends it). A binder such as X2's tells the view where the drag
        // is through `drag_over` and may send no leave before the drop.
        let mut rt = scene("c2-three-columns-light");
        let v = rt.state.c2.columns.column(1).unwrap().collection.viewport();
        let at = (v.x + v.w / 2.0, v.y + 300.0);
        rt.state
            .c2
            .columns
            .drag_over(Some(strata_components::contract::Point::new(at.0, at.1)));
        settle(&mut rt, 3);
        assert_eq!(rt.state.c2.columns.drop_column(), Some(1));
        assert!(info(&rt, &keys::wash("c2-col-2")).is_some());
        rt.drop_files(vec!["/tmp/a.txt".into()], Some(at));
        settle(&mut rt, 3);
        assert_eq!(rt.state.c2.intent, "Dropped 1 into Documents");
        assert_eq!(rt.state.c2.columns.drop_column(), None);
        assert!(info(&rt, &keys::wash("c2-col-2")).is_none());
    }

    #[test]
    fn the_wash_clears_when_the_drag_leaves_the_column_for_the_blank_strip() {
        // operations/drag-and-drop/column-autoscroll: The highlight ... clears when the drag
        // leaves the column.
        let mut rt = scene("c2-three-columns-light");
        let v = rt.state.c2.columns.column(1).unwrap().collection.viewport();
        drag_files(&mut rt, v.x + v.w / 2.0, v.y + 300.0);
        settle(&mut rt, 3);
        assert_eq!(rt.state.c2.columns.drop_column(), Some(1));
        let (bx, by) = at(&rt, &keys::blank("c2"));
        rt.drag_files_to(Some((bx, by)));
        settle(&mut rt, 3);
        assert_eq!(
            rt.state.c2.columns.drop_column(),
            None,
            "over the blank strip"
        );
        assert!(info(&rt, &keys::wash("c2-col-2")).is_none());
        rt.drag_files_leave();
        rt.frame(DT);
    }

    #[test]
    fn a_closed_column_fades_as_it_narrows() {
        // browser/view-modes/columns: A column closed with nothing replacing it narrows and fades
        // out over 220 ms.
        let mut rt = scene("c2-three-columns-light");
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.listings.0.truncate(2);
            g.c2.columns.truncate(2, &look, tx);
        });
        settle(&mut rt, 4);
        let leaving = rt
            .elements()
            .into_iter()
            .find(|e| e.exiting && e.key.as_deref() == Some("c2-slot-3"))
            .expect("Reports' column leaves on a transition");
        assert!(
            leaving.opacity > 0.0 && leaving.opacity < 1.0,
            "fading: {}",
            leaving.opacity
        );
        let (bx, ..) = at(&rt, &keys::blank("c2"));
        assert!(
            bx < 900.0,
            "the slot narrows: the blank strip moved left to {bx}"
        );
    }

    #[test]
    fn a_middle_press_in_another_column_leaves_the_active_column() {
        // browser/view-modes/columns: the active column changes with primary and secondary
        // presses; a middle press (a new tab, autoscroll) leaves it.
        let mut rt = scene("c2-three-columns-light");
        assert_eq!(rt.state.c2.columns.active(), 1);
        let [x, y, _, h] = info(&rt, &row("Home", "Music")).expect("row").visible;
        rt.button_down(x + 40.0, y + h / 2.0, rune::PointerButton::Middle);
        rt.frame(DT);
        rt.button_up(x + 40.0, y + h / 2.0, rune::PointerButton::Middle);
        settle(&mut rt, 2);
        assert_eq!(rt.state.c2.intent, "New tab for Music");
        assert_eq!(rt.state.c2.columns.active(), 1);
    }

    #[test]
    fn type_ahead_finds_an_entry_in_the_active_column_only() {
        // C1c's type-ahead, opt-in, in the column that holds the cursor.
        let mut rt = scene("c2-three-columns-light");
        rt.state.c2.letters = strata_components::collection::keyboard::Letters::TypeAhead;
        tap(&mut rt, &keys::title("c2-col-1"));
        key(&mut rt, Key::Text("t".into()));
        assert_eq!(cursor(&rt, 0), Some(id("Home", "todo.md")));
        assert_eq!(
            selected(&rt, 1),
            [id("Documents", "Reports")],
            "Documents untouched"
        );
    }

    #[test]
    fn a_sibling_fades_in_its_place_and_a_new_column_slides_in() {
        // browser/view-modes/columns: With a folder open, opening a sibling fades the new column
        // in where the old one stood, with no 24 px slide. Deeper columns of the old branch
        // shrink away. (What is drawn a few frames in.)
        let mut rt = scene("c2-three-columns-light");
        let home = id_in("Home", "Pictures");
        let old_x = info(&rt, "c2-slot-2").expect("Documents' shell").rect[0];
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            d.listings.0.truncate(1);
            let listed = strata_gallery::scenes::c2::Listed {
                name: "Pictures".into(),
                source: strata_gallery::scenes::c2::listing_of("Pictures"),
                version: 1,
                due: None,
                from: Some(home),
            };
            d.listings.0.push(listed);
            d.columns.swap_from(
                1,
                strata_components::columns::Opening::new(
                    &d.listings.0[1].source,
                    strata_components::collection::selection::Listing::Complete,
                ),
                strata_components::columns::Focus::Column,
                &look,
                tx,
            );
        });
        settle(&mut rt, 3);
        let body = info(&rt, "c2-col-4-body").expect("the new body");
        assert!(body.opacity < 1.0, "fading in: {}", body.opacity);
        assert_eq!(
            body.rect[0], old_x,
            "where the old one stood, with no slide"
        );
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.exiting && e.key.as_deref() == Some("c2-slot-3")),
            "Reports shrinks away"
        );
        // A new column after the last slides in from the left as it fades.
        tap_name(&mut rt, &row("Pictures", "Holidays"));
        rt.frame(DT);
        let shell = info(&rt, "c2-slot-5").expect("the new shell");
        assert!(shell.opacity < 1.0);
        assert!(
            shell.rect[0] < 600.0,
            "it slides from the left: {}",
            shell.rect[0]
        );
    }

    /// The strip's offset after each of `frames` frames.
    fn offsets(rt: &mut Runtime<Gallery>, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|_| {
                rt.frame(DT);
                strip_offset(rt)
            })
            .collect()
    }

    /// `v` moves in one motion: once it starts, it moves every frame until it is within a
    /// point of where it ends, never stalling (a glide restarted from rest stands still a frame).
    fn moves_without_stalls(v: &[f32]) {
        let (first, last) = (v[0], v[v.len() - 1]);
        let start = v
            .iter()
            .position(|o| (o - first).abs() > 0.5)
            .unwrap_or(v.len());
        let stop = v
            .iter()
            .position(|o| (o - last).abs() < 1.0)
            .unwrap_or(v.len());
        for i in start.max(1)..stop {
            assert!((v[i] - v[i - 1]).abs() > 0.25, "stalls at frame {i}: {v:?}");
        }
    }

    /// The overflow scene with its deepest column active and the strip scrolled back to the
    /// start, so that column is fully clipped.
    fn deepest_clipped() -> (Runtime<Gallery>, f32) {
        let mut rt = scene("c2-overflow-light");
        tap(&mut rt, &keys::title("c2-col-7"));
        settle(&mut rt, 30);
        let end = strip_offset(&rt);
        rt.update(|_, tx| tx.scroll_to_instant(keys::strip("c2"), 0.0, 0.0));
        settle(&mut rt, 5);
        assert_eq!(strip_offset(&rt), 0.0);
        (rt, end)
    }

    #[test]
    fn holding_down_glides_the_strip_in_one_motion() {
        // browser/view-modes/columns: A keyboard move reveals a column clipped by up to 4 px
        // fully (and key repeat keeps the reveal's glide going rather than restarting it).
        let (mut rt, end) = deepest_clipped();
        key_down(&mut rt, Key::Down);
        let single = offsets(&mut rt, 30);
        let (mut rt, _) = deepest_clipped();
        let repeat: Vec<f32> = (0..30)
            .map(|f| {
                if f % 2 == 0 && f < 20 {
                    key_down(&mut rt, Key::Down);
                }
                rt.frame(DT);
                strip_offset(&rt)
            })
            .collect();
        moves_without_stalls(&repeat);
        for (f, (a, b)) in single.iter().zip(&repeat).enumerate() {
            assert!(
                (a - b).abs() < 2.0,
                "frame {f}: one key {a}, held {b}\n{single:?}\n{repeat:?}"
            );
        }
        assert!((repeat[29] - end).abs() < 0.5, "{repeat:?}");
    }

    #[test]
    fn holding_left_glides_the_strip_back_in_one_motion() {
        // browser/view-modes/columns: A keyboard move reveals a column clipped ... fully, Left
        // held from the deepest column back to the first.
        for period in [2, 3] {
            let mut rt = scene("c2-overflow-light");
            tap(&mut rt, &keys::title("c2-col-7"));
            settle(&mut rt, 30);
            let v: Vec<f32> = (0..60)
                .map(|f| {
                    if f % period == 0 && f < 12 * period {
                        key_down(&mut rt, Key::Left);
                    }
                    rt.frame(DT);
                    strip_offset(&rt)
                })
                .collect();
            assert_eq!(rt.state.c2.columns.active(), 0);
            assert!(
                v.windows(2).all(|w| w[1] <= w[0] + 0.25),
                "back only: {v:?}"
            );
            moves_without_stalls(&v);
            assert_eq!(v[59], 0.0);
        }
    }

    /// Right into folder `name` of Home, opened with its first entry, on a 500 pt strip whose
    /// second column is clipped: the strip's offsets as the new column is revealed, with Slow's
    /// listing arriving after `delay` seconds.
    fn right_into(name: &str, delay: f64) -> (Runtime<Gallery>, Vec<f32>) {
        let mut rt = scene("c2-home-light");
        rt.state.c2.width = Some(500.0);
        settle(&mut rt, 5);
        tap(&mut rt, &keys::title("c2-col-1"));
        let folder = id_in("Home", name);
        rt.update(|g, _| {
            let d = &mut g.c2;
            let src = &d.listings.0[0].source;
            let c = &mut d.columns.column_mut(0).expect("Home").collection;
            let i = src.index_of(folder).expect("folder");
            let _ = c.selection.select_only(src, i);
            c.note_selection(src);
        });
        settle(&mut rt, 5);
        assert_eq!(strip_offset(&rt), 0.0);
        key_down(&mut rt, Key::Right);
        rt.frame(DT);
        rt.update(move |g, _| {
            let now = g.c2.clock;
            if let Some(l) = g.c2.listings.0.get_mut(1)
                && l.due.is_some()
            {
                l.due = Some(now + delay);
            }
        });
        let v = offsets(&mut rt, 30);
        (rt, v)
    }

    #[test]
    fn a_late_listing_in_the_active_column_leaves_the_strips_glide_alone() {
        // browser/view-modes/columns: If that column is still loading, the cursor lands when it
        // finishes (in the column, without moving the strip: the reveal's glide goes on as if
        // the listing had not come).
        let (rt, late) = right_into("Slow", 0.05);
        assert_eq!(
            cursor(&rt, 1),
            Some(id("Slow", "late-0001.txt")),
            "the cursor landed"
        );
        let (rt, never) = right_into("Slow", 100.0);
        assert_eq!(cursor(&rt, 1), None);
        for (f, (a, b)) in late.iter().zip(&never).enumerate() {
            assert!(
                (a - b).abs() < 0.5,
                "frame {f}: {a} vs {b}\n{late:?}\n{never:?}"
            );
        }
    }

    #[test]
    fn right_into_a_clipped_new_column_glides_the_strip() {
        // browser/view-modes/columns: A keyboard move reveals a column clipped ... fully, on the
        // reveal's glide: Right into a folder opened with the cursor on its first entry.
        let (rt, v) = right_into("Documents", 0.0);
        assert_eq!(cursor(&rt, 1), Some(id("Documents", "Reports")));
        assert!(v[0] < 60.0, "gliding, not jumping: {v:?}");
        moves_without_stalls(&v);
        assert!((v[29] - 100.0).abs() < 0.5, "{v:?}");
    }

    #[test]
    fn back_to_a_clipped_ancestor_glides_the_strip() {
        // browser/view-modes/columns: Back, Forward, or Alt+Up to an ancestor selects the folder
        // you came from ... scrolled into view (the strip glides to the ancestor's column).
        let (mut rt, _) = deepest_clipped();
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            let came_from = strata_components::columns::ColumnSources::open_child(&d.listings, 4);
            d.listings.0.truncate(5);
            d.columns.return_to(4, &d.listings, came_from, &look, tx);
        });
        let v = offsets(&mut rt, 40);
        assert!(v[0] < 300.0, "gliding, not jumping: {v:?}");
        moves_without_stalls(&v);
        assert!((v[39] - 540.0).abs() < 0.5, "{v:?}");
    }

    #[test]
    fn a_wheel_during_the_reveals_recheck_wins() {
        // browser/view-modes/columns: Horizontal touchpad gestures scroll the strip (and a reveal
        // worked out again while the drawer opens doesn't take the strip back from them).
        let mut rt = scene("c2-overflow-light");
        rt.update(|g, _| g.c2.drawer = Some(strata_components::preview_drawer::DrawerState::new()));
        settle(&mut rt, 20);
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.listings.0.truncate(6);
            g.c2.columns.truncate(6, &look, tx);
            g.c2.drawer.as_mut().expect("drawer").open(&look, tx);
        });
        settle(&mut rt, 6);
        let (x, y) = at(&rt, "c2-slot-1");
        rt.wheel_xy(x, y, 400.0, 0.0);
        rt.frame(DT);
        let wheeled = strip_offset(&rt);
        let v = offsets(&mut rt, 40);
        assert!(
            v.iter().all(|o| (o - wheeled).abs() < 1.0),
            "from {wheeled}: {v:?}"
        );
    }

    #[test]
    fn a_wheel_in_the_same_frame_as_a_key_wins() {
        // browser/view-modes/columns: Horizontal touchpad gestures scroll the strip, even one
        // that lands with a key that reveals a column.
        let (mut rt, _) = deepest_clipped();
        rt.update(|_, tx| tx.scroll_to_instant(keys::strip("c2"), 600.0, 0.0));
        settle(&mut rt, 5);
        assert_eq!(strip_offset(&rt), 600.0);
        let (x, y) = at(&rt, "c2-slot-3");
        key_down(&mut rt, Key::Down);
        rt.wheel_xy(x, y, 200.0, 0.0);
        let v = offsets(&mut rt, 40);
        // The key's glide shares the wheel's frame (Rune applies requests after the wheel), and
        // is stopped at the next tick: one step on from the wheel's 400 at most.
        assert!(v[0] < 500.0, "the wheel took it back from 600: {v:?}");
        assert!(
            v.iter().all(|o| (o - v[0]).abs() < 0.5),
            "and it stays: {v:?}"
        );
    }

    #[test]
    fn a_reveal_follows_the_strips_width_for_half_a_second_only() {
        // browser/view-modes/columns: A keyboard move reveals a column ... fully, worked out again
        // while the strip's width moves within half a second of the move, and no longer: a
        // window resized on and on after it isn't held to that column.
        let mut rt = scene("c2-overflow-light");
        tap(&mut rt, &keys::title("c2-col-7"));
        settle(&mut rt, 40);
        key_down(&mut rt, Key::Down);
        let mut width = 960.0;
        let mut whole_early = true;
        for f in 0..90 {
            if f % 3 == 0 {
                width -= 4.0;
                rt.update(move |g, _| g.c2.width = Some(width));
            }
            rt.frame(DT);
            if f == 24 {
                // A glide chasing a moving width trails it by a fraction of a point.
                let (w, shown) = shown(&rt, 6);
                whole_early = shown > w - 2.0;
            }
        }
        assert!(whole_early, "followed within half a second");
        let (w, shown) = shown(&rt, 6);
        assert!(shown < w - 20.0, "not after it: {shown} of {w}");
    }

    #[test]
    fn a_column_opened_with_its_first_entry_glides_into_view() {
        // browser/view-modes/columns: A keyboard move reveals a column clipped ... fully: here a
        // host opens a folder in a new column with the cursor on its first entry (Return, an
        // intent), outside the strip's keys.
        let mut rt = scene("c2-home-light");
        rt.state.c2.width = Some(500.0);
        settle(&mut rt, 5);
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            d.listings.0.truncate(1);
            d.listings.0.push(strata_gallery::scenes::c2::Listed {
                name: "Music".into(),
                source: strata_gallery::scenes::c2::listing_of("Music"),
                version: 1,
                due: None,
                from: Some(id_in("Home", "Music")),
            });
            d.columns.truncate(1, &look, tx);
            d.columns.push(
                strata_components::columns::Opening::new(
                    &d.listings.0[1].source,
                    strata_components::collection::selection::Listing::Complete,
                ),
                strata_components::columns::Focus::FirstEntry,
                &look,
                tx,
            );
        });
        let v = offsets(&mut rt, 30);
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert!(v[0] < 60.0, "gliding, not jumping: {v:?}");
        assert!((v[29] - 100.0).abs() < 0.5, "{v:?}");
    }

    #[test]
    fn forward_to_a_deeper_open_column_glides_the_strip_to_it() {
        // browser/view-modes/columns: Back, Forward, or Alt+Up to an ancestor selects the folder
        // you came from ... (an ancestor deeper than the column holding the keys: the strip
        // glides to it).
        let (mut rt, _) = deepest_clipped();
        tap(&mut rt, &keys::title("c2-col-1"));
        settle(&mut rt, 20);
        assert_eq!(rt.state.c2.columns.active(), 0);
        assert_eq!(strip_offset(&rt), 0.0);
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            let came_from = strata_components::columns::ColumnSources::open_child(&d.listings, 4);
            d.listings.0.truncate(5);
            d.columns.return_to(4, &d.listings, came_from, &look, tx);
        });
        let v = offsets(&mut rt, 40);
        assert_eq!(rt.state.c2.columns.active(), 4);
        assert!(v[0] < 300.0, "gliding, not jumping: {v:?}");
        assert!((v[39] - 540.0).abs() < 0.5, "{v:?}");
    }
}

/// X2's hook: `ColumnsView::drop_action` answers for each column's folder rows and background,
/// with the column's depth, and for the blank space after the last column.
mod c2_drop_action {
    use std::cell::RefCell;

    use rune::{Cx, El, Env, Mods, Runtime, col};
    use strata_components::contract::{
        Dnd, DropAction, DropActions, DropOffer, DropZone, EntryId, URI_LIST,
    };
    use strata_gallery::{Gallery, scenes};

    type Asked = Option<(usize, Option<EntryId>)>;
    /// A zone the binder attached: its key, its scope, the zone.
    type Attached = (String, Option<String>, DropZone<Gallery>);

    thread_local! {
        static ZONES: RefCell<Vec<Attached>> = const { RefCell::new(Vec::new()) };
        static ASKED: RefCell<Vec<Asked>> = const { RefCell::new(Vec::new()) };
    }

    /// Records each zone the binder attaches, the latest for each key and scope: rows are
    /// memoised, so a view run that rebuilds none attaches none.
    fn view(g: &Gallery, _: &mut Cx<Gallery>) -> El {
        let dnd = Dnd::custom_keyed(
            |el, key, zone| {
                ZONES.with(|z| {
                    let mut z = z.borrow_mut();
                    let (k, s) = (key.key().to_owned(), key.scope().map(str::to_owned));
                    z.retain(|(k2, s2, _)| !(*k2 == k && *s2 == s));
                    z.push((k, s, zone));
                });
                el
            },
            |el, _, _| el,
        );
        let strip = scenes::c2::columns_view()
            .dnd(dnd)
            .drop_action(|_: &Gallery, at: Asked, _, _| {
                ASKED.with(|a| a.borrow_mut().push(at));
                at.map(|_| DropAction::Copy)
            })
            .build(g);
        col().w_full().h_full().child(strip)
    }

    fn mount() -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == "c2-three-columns-light")
            .map(|(_, f)| f)
            .expect("the scene");
        let mut g = Gallery::default();
        f(&mut g);
        let mut rt = Runtime::new(g, view, strata_gallery::SIZE, Env::new(false, 1.0));
        for i in 0..30 {
            rt.frame(f64::from(i) / 60.0);
        }
        rt
    }

    fn asked(zone: &DropZone<Gallery>, g: &Gallery) -> (Option<DropAction>, Asked) {
        ASKED.with(|a| a.borrow_mut().clear());
        let offer = DropOffer::new(vec![URI_LIST.into()], DropActions::COPY | DropActions::MOVE);
        let action = zone.choose(g, &offer, Mods::default());
        let at = ASKED
            .with(|a| a.borrow().last().copied())
            .expect("the hook was asked");
        (action, at)
    }

    #[test]
    fn the_hook_answers_for_rows_backgrounds_and_the_blank_strip_by_depth() {
        let rt = mount();
        let zones = ZONES.with(|z| z.borrow().clone());
        let column_keys: Vec<String> = (0..rt.state.c2.columns.len())
            .filter_map(|d| rt.state.c2.columns.column_key(d))
            .collect();
        assert!(column_keys.len() >= 3, "{column_keys:?}");
        let mut rows = 0;
        for (depth, key) in column_keys.iter().enumerate() {
            let (_, _, bg) = zones
                .iter()
                .find(|(k, s, _)| k == key && s.is_none())
                .unwrap_or_else(|| panic!("column {depth}'s background"));
            assert_eq!(
                asked(bg, &rt.state),
                (Some(DropAction::Copy), Some((depth, None)))
            );
            if let Some((row, scope, zone)) = zones
                .iter()
                .find(|(_, s, _)| s.as_deref() == Some(key.as_str()))
            {
                assert_eq!(scope.as_deref(), Some(key.as_str()));
                let (_, at) = asked(zone, &rt.state);
                let id = row
                    .strip_prefix("row-")
                    .and_then(|n| n.parse().ok())
                    .map(EntryId::new);
                assert_eq!(at, Some((depth, id)), "{row}");
                rows += 1;
            }
        }
        assert!(rows >= 2, "folder rows in several columns");
        let (_, _, blank) = zones
            .iter()
            .find(|(k, _, _)| *k == format!("{}-blank", scenes::c2::KEY))
            .expect("the blank strip");
        assert_eq!(
            asked(blank, &rt.state),
            (None, None),
            "the blank strip takes no drop"
        );
    }

    /// A drag the blank strip refuses still drives the column wash and autoscroll (Strata's
    /// motion controller follows any drag): the refusal keeps the last point, and only
    /// leaving clears it.
    #[test]
    fn a_refused_drag_over_the_blank_strip_keeps_the_column_wash() {
        use strata_components::contract::DropHover;
        let mut rt = mount();
        let blank = ZONES.with(|z| {
            z.borrow()
                .iter()
                .find(|(k, _, _)| *k == format!("{}-blank", scenes::c2::KEY))
                .map(|(_, _, z)| z.clone())
                .expect("the blank strip")
        });
        let key = rt.state.c2.columns.column_key(1).expect("column 1");
        let [x, y, w, h] = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key.as_str()))
            .expect("column 1 drawn")
            .rect;
        let mut at = rune::PointerEvent::default();
        (at.window_x, at.window_y) = (x + w / 2.0, y + h / 2.0);
        let offer = DropOffer::new(vec![URI_LIST.into()], DropActions::MOVE);
        let frames = |rt: &mut Runtime<Gallery>| {
            for _ in 0..3 {
                rt.frame(1.0 / 60.0);
            }
        };
        rt.update(|g, tx| blank.moved(g, at, tx));
        frames(&mut rt);
        assert_eq!(rt.state.c2.columns.drop_column(), Some(1));
        rt.update(|g, tx| blank.hover(g, DropHover::Rejecting, &offer, tx));
        frames(&mut rt);
        assert_eq!(
            rt.state.c2.columns.drop_column(),
            Some(1),
            "refused, still over it"
        );
        rt.update(|g, tx| blank.hover(g, DropHover::Left, &offer, tx));
        frames(&mut rt);
        assert_eq!(rt.state.c2.columns.drop_column(), None, "gone");
    }
}
