// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const SHIFT: ModifierType = ModifierType::SHIFT_MASK;
const ALT: ModifierType = ModifierType::ALT_MASK;

fn peek_has_focus(fixture: &KeyboardFixture) -> bool {
    let focused = gtk::prelude::RootExt::focus(&fixture.window);
    std::iter::successors(focused, gtk::Widget::parent)
        .any(|widget| widget.has_css_class("peek-popover"))
}

/// Presses `key` until the cursor stops moving, returning early once it is on `target`.
fn press_until_stuck(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    key: Key,
    target: Option<&str>,
) -> bool {
    for _ in 0..24 {
        if target.is_some_and(|target| focused_name(browser) == target) {
            return true;
        }
        let before = focused_name(browser);
        fixture.press(key, NONE);
        if focused_name(browser) == before {
            return false;
        }
    }
    false
}

fn focus_icon(fixture: &KeyboardFixture, browser: &crate::app::Browser, name: &str) {
    focus_files(fixture);
    for key in [Key::k, Key::h] {
        press_until_stuck(fixture, browser, key, None);
    }
    for _ in 0..30 {
        if focused_name(browser) == name {
            return;
        }
        let reached = [Key::l, Key::h]
            .into_iter()
            .any(|key| press_until_stuck(fixture, browser, key, Some(name)));
        if reached || focused_name(browser) == name {
            return;
        }
        let before = focused_name(browser);
        fixture.press(Key::j, NONE);
        assert_ne!(
            focused_name(browser),
            before,
            "could not reach {name}, stopped on {before}"
        );
    }
    panic!(
        "could not reach {name}, stopped on {}",
        focused_name(browser)
    );
}

fn display_path(location: Option<Location>) -> Option<String> {
    location.map(|location| location.display_path())
}

fn search_hit_name(fixture: &KeyboardFixture) -> Option<String> {
    fixture
        .view
        .selected_search_result()
        .map(|entry| entry.display_name)
}

fn depth_and_position(browser: &crate::app::Browser) -> Option<(usize, usize)> {
    browser
        .focused_item()
        .map(|(depth, position, _)| (depth, position))
}

struct IconMotion {
    fixture: KeyboardFixture,
    browser: Rc<crate::app::Browser>,
    origin: Option<Location>,
    opened: Rc<RefCell<Vec<Location>>>,
}

impl IconMotion {
    fn new() -> Self {
        let fixture = KeyboardFixture::new();
        let preferences = PreferenceManager::shared();
        preferences.set_tenxer_mode(true);
        preferences.set_folder_peeking(false);
        preferences.set_single_click_previews(true);
        let browser = fixture.view.browser();
        let root = fixture._directory.path();
        std::fs::create_dir(root.join("nest")).expect("nest");
        std::fs::write(root.join("nest/inside.txt"), b"inside").expect("inside");
        std::fs::create_dir(root.join("empty")).expect("empty");
        for index in 0..12 {
            std::fs::write(root.join(format!("tile-{index:02}.txt")), b"t").expect("tile");
        }
        fixture.view.refresh();
        wait_loaded(&browser, 0);
        PreferenceManager::shared().set_icons_thumbnail_size(48);
        fixture.view.set_view_mode(BrowserMode::Icons);
        focus_files(&fixture);
        wait_until(|| {
            source_names(&browser)
                .iter()
                .any(|name| name == "tile-00.txt")
        });
        let origin = browser.active_location();
        Self {
            fixture,
            browser,
            origin,
            opened: Rc::default(),
        }
    }

    fn press(&self, key: Key) {
        self.fixture.press(key, NONE);
    }

    fn cursor(&self) -> String {
        focused_name(&self.browser)
    }

    fn sidebar_focused(&self) -> bool {
        sidebar_has_focus(&self.fixture)
    }

    fn peek_open(&self) -> bool {
        self.fixture.view.widget().has_css_class("peek-open")
    }

    fn keyboard_navigation(&self) -> bool {
        self.fixture
            .view
            .widget()
            .has_css_class("keyboard-navigation")
    }

    fn select_tile_04(&self, settle: u64) {
        select_named(&self.fixture, "tile-04.txt");
        pump(settle);
    }

    fn press_and_load(&self, key: Key, modifiers: ModifierType) {
        self.fixture.press(key, modifiers);
        wait_loaded(&self.browser, 0);
    }

    fn assert_in(&self, name: &str) {
        assert!(location_ends_with(self.browser.active_location(), name));
    }

    fn assert_at_origin(&self) {
        assert_eq!(self.browser.active_location(), self.origin);
    }

    /// Presses one alias from tile-04 and returns the tile it lands on.
    fn alias_landing(&self, key: Key) -> String {
        self.select_tile_04(50);
        assert_eq!(self.cursor(), "tile-04.txt", "{key:?}");
        assert!(self.fixture.press(key, NONE), "{key:?}");
        pump(30);
        assert_eq!(
            (
                self.browser.active_location(),
                self.fixture.view.item_view_has_focus(),
                self.sidebar_focused(),
                self.fixture.preview.is_open(),
                preview_has_focus(&self.fixture),
            ),
            (self.origin.clone(), true, false, false, false),
            "{key:?}: (location, files focused, sidebar focused, preview open, \
             moved into the preview)"
        );
        self.cursor()
    }

    fn letter_arrow_and_keypad_aliases_land_together(&self) {
        for (letter, arrow, keypad) in [
            (Key::h, Key::Left, Key::KP_Left),
            (Key::j, Key::Down, Key::KP_Down),
            (Key::k, Key::Up, Key::KP_Up),
            (Key::l, Key::Right, Key::KP_Right),
        ] {
            let landed: Vec<_> = [letter, arrow, keypad]
                .into_iter()
                .map(|key| self.alias_landing(key))
                .collect();
            assert!(
                landed.iter().all(|name| name == &landed[0]),
                "{letter:?} landed on {landed:?}"
            );
        }
        self.select_tile_04(50);
        self.press(Key::Down);
        pump(30);
        assert_ne!(
            self.cursor(),
            "tile-04.txt",
            "Down leaves the starting tile"
        );
    }

    fn record_open_requests(&self) {
        let record = self.opened.clone();
        self.browser.observe(move |event| {
            if let BrowserEvent::OpenRequested { location } = event {
                record.borrow_mut().push(location.clone());
            }
        });
    }

    fn nothing_launched(&self) -> bool {
        self.opened.borrow().is_empty()
    }

    fn tile_motion_stays_out_of_an_open_preview(&self) {
        self.select_tile_04(50);
        self.record_open_requests();
        self.fixture
            .preview
            .show(self.browser.focused_entry().expect("text file"), Some(0));
        wait_until(|| self.fixture.preview.is_open());
        for key in [Key::l, Key::Right, Key::h, Key::j] {
            self.press(key);
            pump(20);
            assert!(!preview_has_focus(&self.fixture), "{key:?}");
            assert_eq!(self.browser.active_location(), self.origin, "{key:?}");
        }
        assert!(
            self.nothing_launched(),
            "tile motion must not launch a file"
        );
    }

    fn list_l_and_right_enter_the_preview(&self) {
        let fixture = &self.fixture;
        fixture.view.set_view_mode(BrowserMode::List);
        wait_loaded(&self.browser, 0);
        pump(100);
        focus_files(fixture);
        self.select_tile_04(100);
        select_named(fixture, "tile-04.txt");
        assert_eq!(self.cursor(), "tile-04.txt");
        for (key, name) in [(Key::l, "l"), (Key::Right, "Right")] {
            assert!(fixture.press(key, NONE));
            assert_eq!(
                self.browser.active_location(),
                self.origin,
                "List {name} on a file changed directory to {:?}",
                display_path(self.browser.active_location())
            );
        }
        assert!(self.nothing_launched());
        wait_until(|| preview_has_focus(fixture));
        self.press(Key::Escape);
        wait_until(|| fixture.view.item_view_has_focus());
    }

    fn icons_l_and_right_do_not_enter_the_preview(&self) {
        self.fixture.view.set_view_mode(BrowserMode::Icons);
        wait_loaded(&self.browser, 0);
        let names = source_names(&self.browser);
        assert!(
            names.iter().any(|name| name == "tile-04.txt"),
            "after Icons {:?}: {names:?}",
            display_path(self.browser.active_location())
        );
        focus_files(&self.fixture);
        self.select_tile_04(80);
        self.press(Key::l);
        self.press(Key::Right);
        assert!(
            self.nothing_launched(),
            "Icons must not keep the List preview-entry hotkey"
        );
        assert!(!preview_has_focus(&self.fixture));
        self.assert_at_origin();
    }

    fn focus_nest(&self) {
        focus_icon(&self.fixture, &self.browser, "nest");
    }

    fn open_and_back_keys_traverse_history(&self) {
        self.focus_nest();
        self.press_and_load(Key::o, NONE);
        self.assert_in("nest");
        self.press_and_load(Key::BackSpace, NONE);
        self.assert_at_origin();
        self.focus_nest();
        self.press_and_load(Key::Return, NONE);
        self.assert_in("nest");
        self.press_and_load(Key::Up, ALT);
        self.assert_at_origin();
        self.focus_nest();
        self.press_and_load(Key::o, NONE);
        self.press_and_load(Key::H, SHIFT);
        self.assert_at_origin();
        self.press_and_load(Key::L, SHIFT);
        self.assert_in("nest");
        self.press_and_load(Key::Left, ALT);
        self.assert_at_origin();
    }

    fn index(&self) -> usize {
        focused_index(&self.browser)
    }

    fn jumps_and_pages_stay_in_the_folder(&self) -> String {
        focus_files(&self.fixture);
        self.press(Key::Home);
        let first = self.cursor();
        self.press(Key::End);
        let last = self.cursor();
        assert_ne!(first, last);
        self.fixture.press(Key::G, SHIFT);
        assert_eq!(self.cursor(), last);
        self.press(Key::Home);
        assert_eq!(self.cursor(), first);
        let start = self.index();
        self.press(Key::Page_Down);
        let paged = self.index();
        self.press(Key::Home);
        self.fixture.press(Key::d, ModifierType::CONTROL_MASK);
        assert_ne!(self.index(), start);
        self.press(Key::Home);
        self.fixture.press(Key::f, ModifierType::CONTROL_MASK);
        assert_eq!(self.index(), paged);
        self.press(Key::h);
        self.press(Key::l);
        self.assert_at_origin();
        first
    }

    fn left_stays_on_the_first_tile(&self, first: &str) {
        self.press(Key::Home);
        for key in [Key::Left, Key::h, Key::KP_Left] {
            self.press(key);
            assert_eq!(self.cursor(), first, "{key:?} stays on the edge");
            assert!(!self.sidebar_focused(), "{key:?}");
            self.assert_at_origin();
        }
    }

    fn an_empty_folder_keeps_every_motion_key(&self) {
        focus_icon(&self.fixture, &self.browser, "empty");
        self.press_and_load(Key::o, NONE);
        assert_eq!(entry_count(&self.browser), 0);
        self.assert_in("empty");
        focus_files(&self.fixture);
        for key in [
            Key::h,
            Key::j,
            Key::k,
            Key::l,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::KP_Left,
            Key::i,
        ] {
            self.press(key);
            pump(20);
            assert_eq!(
                (
                    location_ends_with(self.browser.active_location(), "empty"),
                    self.sidebar_focused(),
                    self.peek_open()
                ),
                (true, false, false),
                "{key:?}: (still in empty, sidebar focused, peek open)"
            );
        }
        self.press_and_load(Key::BackSpace, NONE);
        self.assert_at_origin();
    }

    fn a_parked_pointer_leaves_motion_to_the_keyboard(&self) {
        select_named(&self.fixture, "tile-01.txt");
        pump(40);
        let before = self.index();
        self.press(Key::j);
        assert_ne!(self.index(), before);
        assert!(self.keyboard_navigation());
        self.fixture
            .view
            .record_pointer_hover((12.0, 24.0), Some(0));
        let parked = self.index();
        self.press(Key::j);
        assert_ne!(self.index(), parked);
        assert!(self.keyboard_navigation());
    }

    fn i_previews_a_tile_in_place(&self) {
        let preview = &self.fixture.preview;
        preview.close();
        pump(40);
        focus_icon(&self.fixture, &self.browser, "tile-02.txt");
        self.press(Key::i);
        wait_until(|| preview.is_open());
        assert!(!self.peek_open());
        assert!(
            self.fixture.view.item_view_has_focus(),
            "Icons i leaves focus on the tile"
        );
        self.assert_at_origin();
        self.press(Key::i);
        assert!(!preview.is_enabled());
    }

    fn i_peeks_a_folder_without_taking_focus(&self) {
        self.focus_nest();
        self.press(Key::j);
        self.press(Key::k);
        self.focus_nest();
        self.press(Key::i);
        pump(80);
        assert!(
            self.peek_open(),
            "i did not peek {}; location {:?}",
            self.cursor(),
            display_path(self.browser.active_location())
        );
        assert_eq!(
            (
                self.fixture.view.item_view_has_focus(),
                peek_has_focus(&self.fixture),
                self.browser.active_location()
            ),
            (true, false, self.origin.clone()),
            "peeking: (files focused, peek focused, location)"
        );
        self.press(Key::i);
        pump(40);
        assert!(!self.peek_open());
    }

    fn focus_recursive_search_hits(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while self
            .fixture
            .view
            .focus_search_results()
            .then(|| self.fixture.view.selected_search_result())
            .flatten()
            .is_none()
        {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
            assert!(
                std::time::Instant::now() < deadline,
                "search results did not focus"
            );
        }
        pump(50);
    }

    /// Presses one alias from the `start` hit and returns the hit it lands on.
    fn search_alias_landing(
        &self,
        key: Key,
        start: (&std::path::Path, &str),
        cursor: Option<(usize, usize)>,
    ) -> Option<String> {
        let view = &self.fixture.view;
        assert!(view.focus_search_result(start.0), "{key:?}");
        pump(40);
        assert_eq!(search_hit_name(&self.fixture).as_deref(), Some(start.1));
        assert!(self.fixture.press(key, NONE), "{key:?}");
        pump(40);
        assert!(
            view.selected_search_results().is_some(),
            "{key:?} dismissed search"
        );
        assert_eq!(
            depth_and_position(&self.browser),
            cursor,
            "{key:?} moved the hidden directory cursor"
        );
        assert_eq!(self.browser.active_location(), self.origin, "{key:?}");
        search_hit_name(&self.fixture)
    }

    fn search_aliases_move_among_hits_without_the_hidden_cursor(&self) {
        let fixture = &self.fixture;
        let preferences = PreferenceManager::shared();
        fixture.view.set_view_mode(BrowserMode::List);
        fixture.view.set_view_mode(BrowserMode::Icons);
        wait_loaded(&self.browser, 0);
        focus_files(fixture);
        let cursor = depth_and_position(&self.browser);
        preferences.set_filter_include_subfolders(true);
        preferences.set_tenxer_mode(false);
        assert!(fixture.view.show_filter_with_query("tile"));
        self.focus_recursive_search_hits();
        assert_eq!(
            depth_and_position(&self.browser),
            cursor,
            "focusing search results moved the hidden directory cursor"
        );
        let start = fixture.view.selected_search_result().expect("search hit");
        preferences.set_tenxer_mode(true);
        let start_name = start.display_name.clone();
        let start_path = start.location.native_path().expect("search hit path");
        wait_until(|| fixture.view.focus_search_result(start_path));
        let search_landed: Vec<_> = [Key::j, Key::Down, Key::KP_Down]
            .into_iter()
            .map(|key| self.search_alias_landing(key, (start_path, &start_name), cursor))
            .collect();
        assert!(
            search_landed.iter().all(|name| name == &search_landed[0]),
            "search aliases diverged: {search_landed:?}"
        );
        assert_ne!(
            search_landed[0].as_deref(),
            Some(start_name.as_str()),
            "Down should move among search icons"
        );
    }
}

#[test]
fn tenxer_icons_move_spatially_open_explicitly_and_peek() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::icon_motion::tenxer_icons_move_spatially_open_explicitly_and_peek",
        || {
            let motion = IconMotion::new();
            motion.letter_arrow_and_keypad_aliases_land_together();
            motion.tile_motion_stays_out_of_an_open_preview();
            motion.list_l_and_right_enter_the_preview();
            motion.icons_l_and_right_do_not_enter_the_preview();
            motion.open_and_back_keys_traverse_history();
            let first = motion.jumps_and_pages_stay_in_the_folder();
            motion.left_stays_on_the_first_tile(&first);
            motion.an_empty_folder_keeps_every_motion_key();
            motion.a_parked_pointer_leaves_motion_to_the_keyboard();
            motion.i_previews_a_tile_in_place();
            motion.i_peeks_a_folder_without_taking_focus();
            motion.search_aliases_move_among_hits_without_the_hidden_cursor();
        },
    );
}

#[test]
fn icons_left_wraps_row_starts_and_leaves_only_the_first_tile() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::icon_motion::icons_left_wraps_row_starts_and_leaves_only_the_first_tile",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            let browser = fixture.view.browser();
            for index in 0..12 {
                std::fs::write(
                    fixture
                        ._directory
                        .path()
                        .join(format!("tile-{index:02}.txt")),
                    b"t",
                )
                .expect("tile");
            }
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            preferences.set_icons_thumbnail_size(48);
            fixture.view.set_view_mode(BrowserMode::Icons);
            wait_loaded(&browser, 0);
            focus_files(&fixture);
            for scoped in [true, false] {
                preferences.set_arrow_navigation_scoped(scoped);
                focus_files(&fixture);
                // Before layout Down steps one tile; the column count then depends on the
                // GTK version's tile width, so any later row start serves.
                wait_until(|| {
                    fixture.press(Key::Home, NONE);
                    gtk::prelude::GtkWindowExt::focus(&fixture.window)
                        .is_some_and(|tile| tile.width() > 0)
                });
                fixture.press(Key::Down, NONE);
                let row_start = focused_index(&browser);
                assert!(row_start > 0, "Down left the first tile focused");
                fixture.press(Key::Left, NONE);
                assert_eq!(
                    (focused_index(&browser), sidebar_has_focus(&fixture)),
                    (row_start - 1, false),
                    "scoped {scoped}: Left at a row start (index, sidebar focused)"
                );
                fixture.press(Key::Home, NONE);
                fixture.press(Key::Left, NONE);
                assert_eq!(
                    sidebar_has_focus(&fixture),
                    !scoped,
                    "scoped {scoped}: Left at the first tile enters the sidebar"
                );
            }
        },
    );
}

#[test]
fn compact_arrows_follow_columns_and_left_leaves_from_the_first_column() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::icon_motion::compact_arrows_follow_columns_and_left_leaves_from_the_first_column",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            let browser = fixture.view.browser();
            for index in 0..40 {
                std::fs::write(
                    fixture
                        ._directory
                        .path()
                        .join(format!("tile-{index:02}.txt")),
                    b"t",
                )
                .expect("tile");
            }
            fixture.view.refresh();
            wait_loaded(&browser, 0);
            fixture.view.set_view_mode(BrowserMode::Compact);
            wait_loaded(&browser, 0);
            for scoped in [true, false] {
                preferences.set_arrow_navigation_scoped(scoped);
                focus_files(&fixture);
                wait_until(|| {
                    fixture.press(Key::Home, NONE);
                    gtk::prelude::GtkWindowExt::focus(&fixture.window)
                        .is_some_and(|cell| cell.width() > 0)
                });
                fixture.press(Key::Down, NONE);
                assert_eq!(focused_index(&browser), 1, "Down moves within the column");
                fixture.press(Key::Right, NONE);
                let next_column = focused_index(&browser);
                assert!(
                    next_column > 2,
                    "Right moved to {next_column}, not the next column"
                );
                fixture.press(Key::Up, NONE);
                assert_eq!(focused_index(&browser), next_column - 1);
                fixture.press(Key::Left, NONE);
                assert_eq!(
                    (focused_index(&browser), sidebar_has_focus(&fixture)),
                    (0, false),
                    "scoped {scoped}: Left returns to the same row of the first column"
                );
                fixture.press(Key::Down, NONE);
                fixture.press(Key::Left, NONE);
                assert_eq!(
                    sidebar_has_focus(&fixture),
                    !scoped,
                    "scoped {scoped}: Left anywhere in the first column enters the sidebar"
                );
            }
        },
    );
}
