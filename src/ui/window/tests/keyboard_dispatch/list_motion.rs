// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const CONTROL: ModifierType = ModifierType::CONTROL_MASK;
const SHIFT: ModifierType = ModifierType::SHIFT_MASK;
const ALT: ModifierType = ModifierType::ALT_MASK;

struct ListMotion {
    fixture: KeyboardFixture,
    browser: Rc<crate::app::Browser>,
    root: std::path::PathBuf,
    origin: Option<Location>,
}

fn seed_motion_files(root: &std::path::Path) {
    std::fs::create_dir(root.join("nest")).expect("nest");
    std::fs::write(root.join("nest/inside.txt"), b"inside").expect("inside");
    std::fs::create_dir(root.join("empty")).expect("empty");
    std::fs::create_dir(root.join("aaa-dir")).expect("group folder");
    std::fs::write(root.join("a.png"), b"p").expect("png");
    std::fs::write(root.join("m.txt"), b"m").expect("text");
    for index in 0..40 {
        std::fs::write(root.join(format!("n{index:02}.txt")), b"n").expect("page file");
    }
}

fn first_is(names: Vec<String>, name: &str) -> bool {
    names.first().is_some_and(|first| first == name)
}

impl ListMotion {
    fn new() -> Self {
        default_recorder_app("text/plain", &glib::user_data_dir().join("launched"));
        let fixture = KeyboardFixture::new();
        let preferences = PreferenceManager::shared();
        preferences.set_tenxer_mode(true);
        preferences.set_columns_mirror_selection(true);
        preferences.set_group_by_type(false);
        preferences.set_single_click_previews(true);
        let browser = fixture.view.browser();
        let root = fixture._directory.path().to_path_buf();
        seed_motion_files(&root);
        fixture.view.refresh();
        wait_loaded(&browser, 0);
        browser.set_folders_first(0, false);
        browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
        wait_until(|| first_is(source_names(&browser), "a.png"));
        let origin = browser.active_location();
        Self {
            fixture,
            browser,
            root,
            origin,
        }
    }

    fn press(&self, key: Key) {
        self.fixture.press(key, NONE);
    }

    fn press_with(&self, key: Key, modifiers: ModifierType) {
        self.fixture.press(key, modifiers);
    }

    fn cursor(&self) -> String {
        focused_name(&self.browser)
    }

    fn index(&self) -> usize {
        focused_index(&self.browser)
    }

    fn depth(&self) -> Option<usize> {
        focused_depth(&self.browser)
    }

    fn sidebar_focused(&self) -> bool {
        sidebar_has_focus(&self.fixture)
    }

    fn wait_listed_first(&self, name: &str) {
        wait_until(|| first_is(list_display_names(&self.fixture.view.widget()), name));
    }

    /// Presses `key` and returns the cursor index it lands on.
    fn index_after(&self, key: Key, modifiers: ModifierType) -> usize {
        self.press_with(key, modifiers);
        self.index()
    }

    fn jk_arrows_and_g_move_the_cursor(&self) {
        self.fixture.view.set_view_mode(BrowserMode::List);
        focus_files(&self.fixture);
        self.wait_listed_first("a.png");
        for (key, expected) in [
            (Key::Home, "a.png"),
            (Key::j, "a.txt"),
            (Key::k, "a.png"),
            (Key::Down, "a.txt"),
            (Key::Up, "a.png"),
        ] {
            self.press(key);
            assert_eq!(self.cursor(), expected, "{key:?}");
        }
        assert_eq!(
            (
                self.fixture.view.item_view_has_focus(),
                self.sidebar_focused()
            ),
            (true, false),
            "(files focused, sidebar focused)"
        );
        self.press(Key::End);
        let last = self.cursor();
        self.press(Key::Home);
        self.press_with(Key::G, SHIFT);
        assert_eq!(self.cursor(), last, "G reaches the last item");
        self.press(Key::Home);
        assert_eq!(self.cursor(), "a.png");
    }

    fn half_and_full_pages_move_down(&self, start: usize) {
        let count = entry_count(&self.browser);
        let half = self.index_after(Key::d, CONTROL);
        assert!(half > start, "Ctrl+D moves down");
        assert!(half + 1 < count, "half page stays inside the listing");
        self.press(Key::Home);
        let page_down = self.index_after(Key::Page_Down, NONE);
        self.press(Key::Home);
        let full = self.index_after(Key::f, CONTROL);
        assert!(full > start && page_down > start);
        assert_eq!(full, page_down, "Ctrl+F and Page Down move one full page");
        if half - start > 1 {
            assert!(full > half, "a full page moves farther than a half page");
        }
    }

    fn paging_clamps_at_both_ends(&self, start: usize) {
        self.press(Key::End);
        let end = self.index();
        for (key, modifiers) in [(Key::d, CONTROL), (Key::Page_Down, NONE), (Key::f, CONTROL)] {
            self.press_with(key, modifiers);
        }
        assert_eq!(self.index(), end, "paging down clamps at the last item");
        let half_up = self.index_after(Key::u, CONTROL);
        assert!(half_up < end, "Ctrl+U moves up");
        self.press(Key::End);
        let page_up = self.index_after(Key::Page_Up, NONE);
        self.press(Key::End);
        assert_eq!(
            self.index_after(Key::b, CONTROL),
            page_up,
            "Ctrl+B and Page Up move one full page"
        );
        self.press(Key::Home);
        for (key, modifiers) in [(Key::u, CONTROL), (Key::Page_Up, NONE), (Key::b, CONTROL)] {
            self.press_with(key, modifiers);
        }
        assert_eq!(self.index(), start, "paging up clamps at the first item");
    }

    fn paging_keys_neither_filter_nor_preview(&self) {
        let fixture = &self.fixture;
        assert_eq!(
            (
                fixture.view.filter_has_focus(),
                fixture.sidebar_toggle.is_active(),
                fixture.preview.is_open()
            ),
            (false, true, false),
            "after paging: (filter focused, sidebar shown, preview open)"
        );
        self.press_with(Key::n, CONTROL);
        assert!(
            !fixture.sidebar_toggle.is_active(),
            "Ctrl+N hides the sidebar"
        );
        assert!(fixture.view.item_view_has_focus());
        self.press_with(Key::n, CONTROL);
        assert!(fixture.sidebar_toggle.is_active(), "Ctrl+N shows it again");
    }

    fn jk_follow_a_descending_sort(&self) {
        let browser = &self.browser;
        browser.set_sort(0, SortKey::Name, SortDirection::Descending);
        wait_until(|| {
            source_names(browser)
                .first()
                .is_some_and(|name| name != "a.png")
        });
        focus_files(&self.fixture);
        self.press(Key::Home);
        let descending = source_names(browser);
        assert_eq!(self.cursor(), descending[0]);
        for (key, expected) in [(Key::j, &descending[1]), (Key::k, &descending[0])] {
            self.press(key);
            assert_eq!(&self.cursor(), expected, "{key:?}");
        }
    }

    fn j_walks_grouped_rows_in_display_order(&self) {
        let preferences = PreferenceManager::shared();
        self.browser
            .set_sort(0, SortKey::Name, SortDirection::Ascending);
        preferences.set_group_by_type(true);
        self.wait_listed_first("aaa-dir");
        let displayed = list_display_names(&self.fixture.view.widget());
        assert_ne!(
            displayed,
            source_names(&self.browser),
            "grouped rows are not source order"
        );
        focus_files(&self.fixture);
        self.press(Key::Home);
        let mut walked = vec![self.cursor()];
        for _ in 1..displayed.len().min(6) {
            self.press(Key::j);
            walked.push(self.cursor());
        }
        assert_eq!(&walked[..], &displayed[..walked.len()]);
        preferences.set_group_by_type(false);
        self.wait_listed_first("a.png");
    }

    fn press_and_load(&self, key: Key, modifiers: ModifierType) {
        self.press_with(key, modifiers);
        wait_loaded(&self.browser, 0);
    }

    fn in_nest(&self) -> bool {
        location_ends_with(self.browser.active_location(), "nest")
    }

    fn at_origin(&self) -> bool {
        self.browser.active_location() == self.origin
    }

    fn l_enters_and_history_keys_traverse(&self) {
        move_to_named(&self.fixture, &self.browser, "nest");
        self.press_and_load(Key::l, NONE);
        assert!(
            self.in_nest(),
            "l from {} stayed at {:?}",
            self.cursor(),
            self.browser
                .active_location()
                .map(|location| location.display_path())
        );
        assert!(!self.sidebar_focused());
        for (key, modifiers, nested) in [
            (Key::H, SHIFT, false),
            (Key::L, SHIFT, true),
            (Key::BackSpace, NONE, false),
        ] {
            self.press_and_load(key, modifiers);
            assert_eq!(
                (self.in_nest(), self.at_origin()),
                (nested, !nested),
                "{modifiers:?}+{key:?}: (in nest, at origin)"
            );
        }
        move_to_named(&self.fixture, &self.browser, "nest");
        self.press_and_load(Key::Right, NONE);
        for (key, nested) in [(Key::Up, false), (Key::Left, true), (Key::Right, false)] {
            self.press_and_load(key, ALT);
            assert_eq!(
                (self.in_nest(), self.at_origin()),
                (nested, !nested),
                "Alt+{key:?}: (in nest, at origin)"
            );
        }
    }

    fn an_empty_folder_keeps_focus_off_the_sidebar(&self) {
        move_to_named(&self.fixture, &self.browser, "empty");
        self.press_and_load(Key::o, NONE);
        assert_eq!(entry_count(&self.browser), 0);
        focus_files(&self.fixture);
        for key in [Key::j, Key::k, Key::l, Key::Right, Key::i, Key::Return] {
            self.press(key);
            assert!(!self.sidebar_focused(), "{key:?}");
        }
        self.press_and_load(Key::h, NONE);
        assert!(self.at_origin());
        assert!(!self.sidebar_focused());
    }

    fn files_keep_focus_off_the_preview(&self) -> bool {
        self.fixture.view.item_view_has_focus() && !preview_has_focus(&self.fixture)
    }

    fn the_cursor_folder_mirrors_into_the_next_column(&self) {
        let fixture = &self.fixture;
        fixture.preview.observe_browser(&self.browser);
        fixture.view.set_view_mode(BrowserMode::Columns);
        wait_loaded(&self.browser, 0);
        assert!(fixture.view.columns_mirror_selection_enabled());
        select_named(fixture, "n39.txt");
        self.press(Key::j);
        assert_eq!(self.cursor(), "nest");
        wait_loaded(&self.browser, 1);
        assert!(
            location_ends_with(self.browser.location_at(1), "nest"),
            "the cursor's folder opens in the next column"
        );
        assert_eq!(self.depth(), Some(0));
        assert!(fixture.view.item_view_has_focus());
    }

    fn i_toggles_the_preview_of_a_file(&self) {
        let preview = &self.fixture.preview;
        self.press(Key::k);
        wait_until(|| preview.is_open());
        assert!(
            self.browser.column_snapshot(1).is_none(),
            "a file under the cursor closes the child column"
        );
        assert!(self.files_keep_focus_off_the_preview());
        self.press(Key::i);
        assert!(!preview.is_enabled(), "i closes the preview");
        self.press(Key::i);
        wait_until(|| preview.is_open());
        assert!(
            self.browser.column_snapshot(1).is_none(),
            "i on a file opens no column"
        );
        assert!(
            self.files_keep_focus_off_the_preview(),
            "i leaves focus in the list"
        );
        self.press(Key::i);
        assert!(!preview.is_enabled(), "a second i closes the preview");
        assert!(self.fixture.view.item_view_has_focus());
    }

    fn keyboard_navigation(&self) -> bool {
        self.fixture
            .view
            .widget()
            .has_css_class("keyboard-navigation")
    }

    /// `i` on a folder opens its column; only a moving pointer claims the
    /// command destination back from the keyboard.
    fn the_pointer_claims_the_destination_only_by_moving(&self) {
        select_named(&self.fixture, "nest");
        self.press(Key::i);
        wait_loaded(&self.browser, 1);
        assert!(location_ends_with(self.browser.location_at(1), "nest"));
        assert_eq!(self.depth(), Some(0));
        let child = self.browser.location_at(1);
        self.press(Key::j);
        pump(200);
        assert_eq!(
            (
                self.browser.location_at(1),
                self.depth(),
                self.keyboard_navigation()
            ),
            (child.clone(), Some(0), true),
            "j keeps the opened column: (child column, cursor depth, keyboard navigation)"
        );
        self.fixture
            .view
            .record_pointer_hover((12.0, 24.0), Some(1));
        assert!(
            !self.keyboard_navigation(),
            "pointer motion claims command destination"
        );
        let before = self.index();
        self.press(Key::k);
        assert_ne!(self.index(), before);
        assert_eq!(self.depth(), Some(0));
        assert!(self.keyboard_navigation());
        self.fixture
            .view
            .record_pointer_hover((12.0, 24.0), Some(1));
        assert!(
            self.keyboard_navigation(),
            "a parked pointer must not reclaim the destination"
        );
        self.press(Key::j);
        assert_eq!(
            (self.depth(), self.browser.location_at(1)),
            (Some(0), child)
        );
    }

    fn l_and_h_enter_and_leave_the_child_column(&self) {
        select_named(&self.fixture, "nest");
        self.press(Key::l);
        wait_loaded(&self.browser, 1);
        assert_eq!(self.depth(), Some(1));
        assert!(!self.sidebar_focused());
        self.press(Key::h);
        wait_until(|| self.browser.column_snapshot(1).is_none());
        assert_eq!(self.depth(), Some(0));
        assert!(!self.sidebar_focused());
    }

    fn an_empty_child_column_opens_nothing_further(&self) {
        select_named(&self.fixture, "empty");
        self.press(Key::Right);
        wait_loaded(&self.browser, 1);
        assert_eq!(
            self.browser.column_snapshot(1).map(|column| column.count),
            Some(0)
        );
        focus_files(&self.fixture);
        self.press(Key::i);
        pump(100);
        assert!(self.browser.column_snapshot(2).is_none());
        self.press(Key::Left);
        wait_until(|| self.browser.column_snapshot(1).is_none());
        assert!(!self.sidebar_focused());
    }

    fn shift_h_closes_and_l_reopens_the_column(&self) {
        select_named(&self.fixture, "nest");
        self.press(Key::Right);
        wait_loaded(&self.browser, 1);
        self.press_and_load(Key::H, SHIFT);
        assert!(self.browser.column_snapshot(1).is_none());
        self.press(Key::L);
        wait_loaded(&self.browser, 1);
        assert!(location_ends_with(self.browser.location_at(1), "nest"));
    }

    fn the_filesystem_root_stays_put(&self) {
        let preferences = PreferenceManager::shared();
        preferences.set_tenxer_mode(false);
        assert!(
            self.fixture.view.columns_mirror_selection_enabled(),
            "turning the mode off restores saved column mirroring"
        );

        self.browser.navigate(Location::local("/"));
        wait_loaded(&self.browser, 0);
        preferences.set_tenxer_mode(true);
        focus_files(&self.fixture);
        let filesystem_root = self.browser.active_location();
        self.press(Key::h);
        self.press(Key::Left);
        pump(100);
        assert_eq!(self.browser.active_location(), filesystem_root);
        assert!(!self.sidebar_focused());
        assert!(
            self.browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
        );
    }

    fn default_chords_return_when_the_mode_is_off(&self) {
        let fixture = &self.fixture;
        PreferenceManager::shared().set_tenxer_mode(false);
        self.browser.navigate(Location::local(&self.root));
        wait_loaded(&self.browser, 0);
        fixture.view.set_view_mode(BrowserMode::List);
        focus_files(fixture);
        let before_duplicate = directory_names(&self.root);
        self.press_with(Key::d, CONTROL);
        wait_until(|| directory_names(&self.root).len() > before_duplicate.len());
        self.press_with(Key::f, CONTROL);
        assert!(
            fixture.view.filter_has_focus(),
            "Ctrl+F filters when the mode is off"
        );
        assert!(fixture.press(Key::Escape, NONE));
        assert!(fixture.sidebar_toggle.is_active());
        self.press_with(Key::b, CONTROL);
        assert!(!fixture.sidebar_toggle.is_active());
        self.press_with(Key::n, CONTROL);
        assert!(
            !fixture.sidebar_toggle.is_active(),
            "Ctrl+N toggles only in 10xer mode"
        );
    }

    fn only_o_and_enter_launch_a_file(&self) {
        PreferenceManager::shared().set_tenxer_mode(true);
        focus_files(&self.fixture);
        move_to_named(&self.fixture, &self.browser, "a.txt");
        let opened = Rc::new(RefCell::new(Vec::new()));
        let record = opened.clone();
        self.browser.observe(move |event| {
            if let BrowserEvent::OpenRequested { location } = event {
                record.borrow_mut().push(location.clone());
            }
        });
        let launched_a = || {
            opened
                .borrow()
                .iter()
                .any(|location| location_ends_with(Some(location.clone()), "a.txt"))
        };
        let stayed = self.browser.active_location();
        self.press(Key::l);
        self.press(Key::Right);
        assert!(
            opened.borrow().is_empty(),
            "plain l / Right must not launch a file"
        );
        assert_eq!(self.browser.active_location(), stayed);
        self.press(Key::Escape);
        wait_until(|| self.fixture.view.item_view_has_focus());
        self.press(Key::o);
        assert!(launched_a(), "o launches the focused file");
        opened.borrow_mut().clear();
        self.press(Key::Return);
        assert!(launched_a(), "Enter launches the focused file");
    }
}

#[test]
fn tenxer_list_and_columns_move_enter_and_traverse_history() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::list_motion::tenxer_list_and_columns_move_enter_and_traverse_history",
        || {
            let motion = ListMotion::new();
            motion.jk_arrows_and_g_move_the_cursor();
            let start = motion.index();
            motion.half_and_full_pages_move_down(start);
            motion.paging_clamps_at_both_ends(start);
            motion.paging_keys_neither_filter_nor_preview();
            motion.jk_follow_a_descending_sort();
            motion.j_walks_grouped_rows_in_display_order();
            motion.l_enters_and_history_keys_traverse();
            motion.an_empty_folder_keeps_focus_off_the_sidebar();

            motion.the_cursor_folder_mirrors_into_the_next_column();
            motion.i_toggles_the_preview_of_a_file();
            motion.the_pointer_claims_the_destination_only_by_moving();
            motion.l_and_h_enter_and_leave_the_child_column();
            motion.an_empty_child_column_opens_nothing_further();
            motion.shift_h_closes_and_l_reopens_the_column();
            motion.the_filesystem_root_stays_put();
            motion.default_chords_return_when_the_mode_is_off();
            motion.only_o_and_enter_launch_a_file();
        },
    );
}
