// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const CONTROL: ModifierType = ModifierType::CONTROL_MASK;
const SHIFT: ModifierType = ModifierType::SHIFT_MASK;

fn tenxer_without_grouping(fixture: &KeyboardFixture) -> Rc<crate::app::Browser> {
    let preferences = PreferenceManager::shared();
    preferences.set_tenxer_mode(true);
    preferences.set_group_by_type(false);
    let browser = fixture.view.browser();
    fixture.shortcuts.observe_browser(&browser);
    PreferenceManager::shared().set_icons_thumbnail_size(48);
    browser
}

fn add_and_enter_empty_list_folder(fixture: &KeyboardFixture, browser: &crate::app::Browser) {
    std::fs::create_dir(fixture._directory.path().join("empty")).expect("empty");
    fixture.view.refresh();
    wait_loaded(browser, 0);
    fixture.view.set_view_mode(BrowserMode::List);
}

fn start_with_an_empty_fill(fixture: &KeyboardFixture, browser: &crate::app::Browser) {
    focus_files(fixture);
    browser.select(0, 0);
    assert!(browser.clear_active_selection());
    focus_files(fixture);
    assert!(browser.selected_positions(0).is_empty());
    assert_eq!(focused_name(browser), "a.txt");
}

fn space_fills_and_advances(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    mode: BrowserMode,
) {
    assert!(fixture.press(Key::space, NONE));
    assert!(!fixture.preview.is_open(), "{mode:?}");
    assert_cursor_fill(browser, "b.txt", &["a.txt"], format!("{mode:?}"));
    if mode == BrowserMode::Columns {
        wait_until(|| focused_widget_shows(&fixture.window, "b.txt"));
    }

    fixture.press(Key::space, NONE);
    assert_cursor_fill(browser, "c.txt", &["a.txt", "b.txt"], format!("{mode:?}"));
    assert!(!fixture.preview.is_open(), "{mode:?}");
}

fn jumps_select_all_and_reset_keep_the_cursor(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    mode: BrowserMode,
) {
    let all = ["a.txt", "b.txt", "c.txt"];
    fixture.press(Key::Home, NONE);
    assert_cursor_fill(browser, "a.txt", &["a.txt", "b.txt"], format!("{mode:?}"));
    fixture.press(Key::a, CONTROL);
    assert_cursor_fill(browser, "a.txt", &all, format!("{mode:?}"));
    fixture.press(Key::End, NONE);
    assert_cursor_fill(browser, "c.txt", &all, format!("{mode:?}"));

    fixture.press(Key::r, CONTROL);
    assert!(!fixture.view.rename_is_active(), "{mode:?}");
    assert!(fill_names(browser).is_empty(), "{mode:?}");
    fixture.press(Key::Home, NONE);
    assert_cursor_fill(browser, "a.txt", &[], format!("{mode:?}"));

    fixture.press(Key::a, CONTROL);
    assert_eq!(fill_names(browser), all, "{mode:?}");
    fixture.press(Key::Down, CONTROL);
    assert_cursor_fill(
        browser,
        "c.txt",
        &all,
        format!("{mode:?} Ctrl+Down keeps the fill"),
    );
    fixture.press(Key::Up, CONTROL);
    assert_cursor_fill(
        browser,
        "a.txt",
        &all,
        format!("{mode:?} Ctrl+Up keeps the fill"),
    );
}

fn icons_right_keeps_the_fill(fixture: &KeyboardFixture, browser: &crate::app::Browser) {
    let mode = BrowserMode::Icons;
    fixture.press(Key::r, CONTROL);
    assert!(fill_names(browser).is_empty(), "{mode:?}");
    fixture.press(Key::Home, NONE);
    fixture.press(Key::space, NONE);
    assert_cursor_fill(browser, "b.txt", &["a.txt"], format!("{mode:?}"));
    wait_until(|| focused_widget_shows(&fixture.window, "b.txt"));
    fixture.press(Key::Right, NONE);
    assert_cursor_fill(
        browser,
        "c.txt",
        &["a.txt"],
        format!("{mode:?} Right moves from b.txt to c.txt and keeps a one-item fill"),
    );
    fixture.press(Key::a, CONTROL);
    fixture.press(Key::r, CONTROL);
    fixture.press(Key::Home, NONE);
    assert_cursor_fill(browser, "a.txt", &[], format!("{mode:?}"));
    wait_until(|| focused_widget_shows(&fixture.window, "a.txt"));
    fixture.press(Key::Right, NONE);
    assert_ne!(focused_name(browser), "a.txt", "{mode:?}");
    assert!(
        fill_names(browser).is_empty(),
        "{mode:?} Right keeps an empty fill"
    );
}

fn empty_folder_has_nothing_to_select(fixture: &KeyboardFixture, browser: &crate::app::Browser) {
    focus_files(fixture);
    fixture.press(Key::space, NONE);
    assert_eq!(
        fixture.shortcuts.feedback_text(),
        "Nothing to select",
        "an empty folder flashes instead of selecting a path marker"
    );
    assert!(browser.selected_entries().is_empty());
    assert!(!fixture.preview.is_open());
    wait_until(|| fixture.shortcuts.feedback_text().is_empty());
    for key in [Key::a, Key::r] {
        fixture.press(key, CONTROL);
        assert_eq!(fixture.shortcuts.feedback_text(), "Nothing to select");
        wait_until(|| fixture.shortcuts.feedback_text().is_empty());
    }

    focus_files(fixture);
    fixture.press(Key::space, NONE);
    assert_eq!(fixture.shortcuts.feedback_text(), "Nothing to select");
    fixture.press(Key::BackSpace, NONE);
    assert!(
        fixture.shortcuts.feedback_text().is_empty(),
        "leaving the empty folder clears the flash"
    );
}

fn ctrl_a_stays_in_the_focused_pane(fixture: &KeyboardFixture, browser: &crate::app::Browser) {
    wait_loaded(browser, 0);
    fixture.view.set_view_mode(BrowserMode::Columns);
    focus_files(fixture);
    move_to_named(fixture, browser, "empty");
    fixture.press(Key::i, NONE);
    wait_loaded(browser, 1);
    fixture.press(Key::a, CONTROL);
    assert!(browser.selected_positions(0).len() > 1);
    assert!(
        browser.selected_positions(1).is_empty(),
        "Ctrl+A stays in the focused pane"
    );
}

#[test]
fn tenxer_space_selects_the_cursor_and_motion_keeps_the_fill() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_fill::tenxer_space_selects_the_cursor_and_motion_keeps_the_fill",
        || {
            let fixture = KeyboardFixture::new();
            let browser = tenxer_without_grouping(&fixture);
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                fixture.view.set_view_mode(mode);
                start_with_an_empty_fill(&fixture, &browser);
                space_fills_and_advances(&fixture, &browser, mode);
                jumps_select_all_and_reset_keep_the_cursor(&fixture, &browser, mode);
                if mode == BrowserMode::Icons {
                    icons_right_keeps_the_fill(&fixture, &browser);
                }
            }

            add_and_enter_empty_list_folder(&fixture, &browser);
            focus_files(&fixture);
            move_to_named(&fixture, &browser, "empty");
            fixture.press(Key::Return, NONE);
            wait_until(|| {
                browser
                    .active_location()
                    .is_some_and(|location| location.display_path().ends_with("empty"))
                    && entry_count(&browser) == 0
            });
            empty_folder_has_nothing_to_select(&fixture, &browser);
            ctrl_a_stays_in_the_focused_pane(&fixture, &browser);
        },
    );
}

#[test]
fn tenxer_columns_click_on_the_cursor_replaces_the_fill() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_fill::tenxer_columns_click_on_the_cursor_replaces_the_fill",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_group_by_type(false);
            fixture.view.set_view_mode(BrowserMode::Columns);
            let browser = fixture.view.browser();
            focus_files(&fixture);
            assert!(browser.clear_active_selection());
            focus_files(&fixture);

            fixture.press(Key::End, NONE);
            assert_eq!(focused_name(&browser), "c.txt");
            fixture.press(Key::space, NONE);
            assert_cursor_fill(&browser, "c.txt", &["c.txt"], "Space");
            fixture.press(Key::Up, NONE);
            assert_cursor_fill(&browser, "b.txt", &["c.txt"], "Up");
            wait_until(|| focused_widget_shows(&fixture.window, "b.txt"));

            assert!(
                press_file_row(&fixture.view.widget(), "b.txt"),
                "b.txt row accepts the click"
            );
            assert_cursor_fill(
                &browser,
                "b.txt",
                &["b.txt"],
                "clicking the cursor replaces the filled selection",
            );
        },
    );
}

struct VisualWalk<'a> {
    fixture: &'a KeyboardFixture,
    browser: Rc<crate::app::Browser>,
    mode: BrowserMode,
    next: Key,
    previous: Key,
}

impl VisualWalk<'_> {
    fn press(&self, key: Key) {
        self.fixture.press(key, NONE);
    }

    fn visual_text(&self) -> Option<String> {
        self.fixture.shortcuts.visual_text()
    }

    fn assert_fill(&self, fill: &[&str], context: &str) {
        assert_eq!(fill_names(&self.browser), fill, "{:?}{context}", self.mode);
    }

    fn assert_cursor_fill(&self, cursor: &str, fill: &[&str], context: &str) {
        assert_cursor_fill(
            &self.browser,
            cursor,
            fill,
            format!("{:?}{context}", self.mode),
        );
    }

    fn wait_icon_shows(&self, name: &str) {
        if self.mode == BrowserMode::Icons {
            wait_until(|| focused_widget_shows(&self.fixture.window, name));
        }
    }

    fn fill_a_and_end_on_e(&self) {
        self.fixture.view.set_view_mode(self.mode);
        focus_files(self.fixture);
        self.browser.select(0, 0);
        self.browser.clear_active_selection();
        focus_files(self.fixture);
        self.press(Key::Home);
        self.press(Key::space);
        self.press(Key::End);
        self.assert_cursor_fill("e.txt", &["a.txt"], "");
    }

    fn v_walks_a_range_and_space_toggles(&self) {
        assert!(self.fixture.press(Key::v, NONE));
        self.assert_fill(&["a.txt", "e.txt"], "");
        assert_eq!(self.visual_text().as_deref(), Some("VISUAL"));
        self.press(self.previous);
        self.press(self.previous);
        self.assert_cursor_fill(
            "c.txt",
            &["a.txt", "c.txt", "d.txt", "e.txt"],
            " v walks a range",
        );
        self.press(Key::space);
        self.assert_cursor_fill("c.txt", &["a.txt", "d.txt", "e.txt"], " Space stays put");
        assert!(!self.fixture.preview.is_open(), "{:?}", self.mode);
    }

    fn escape_ends_the_range_and_keeps_the_fill(&self) {
        self.press(Key::Escape);
        assert_eq!(self.visual_text(), None, "{:?}", self.mode);
        self.assert_fill(&["a.txt", "d.txt", "e.txt"], "");
        self.press(self.previous);
        self.assert_cursor_fill(
            "b.txt",
            &["a.txt", "d.txt", "e.txt"],
            " motion after Escape keeps the fill",
        );
    }

    fn shift_v_subtracts_the_walked_span(&self) {
        assert!(self.fixture.press(Key::V, SHIFT));
        assert_eq!(self.visual_text().as_deref(), Some("UNSET"));
        self.press(self.next);
        self.press(self.next);
        self.assert_cursor_fill("d.txt", &["a.txt", "e.txt"], " V subtracts the walked span");
        self.fixture.press(Key::V, SHIFT);
        assert_eq!(self.visual_text(), None, "{:?}", self.mode);
        self.press(self.next);
        self.assert_fill(&["a.txt", "e.txt"], "");
    }

    fn a_new_v_anchors_at_the_cursor(&self) {
        self.press(Key::v);
        self.press(self.previous);
        self.assert_fill(
            &["a.txt", "d.txt", "e.txt"],
            " a new v anchors at the cursor",
        );
        self.press(Key::Escape);
        self.assert_fill(&["a.txt", "d.txt", "e.txt"], "");
        self.press(Key::Escape);
        assert!(
            fill_names(&self.browser).is_empty(),
            "{:?} the next Escape clears the fill",
            self.mode
        );
    }

    fn shift_arrows_never_select(&self) {
        self.press(Key::Home);
        self.press(Key::space);
        self.wait_icon_shows("b.txt");
        for arrow in [Key::Down, Key::Up, Key::Right, Key::Left] {
            let mode = self.mode;
            assert!(self.fixture.press(arrow, SHIFT), "{mode:?} Shift+{arrow:?}");
            self.assert_cursor_fill(
                "b.txt",
                &["a.txt"],
                &format!(" Shift+{arrow:?} does not select"),
            );
        }
        self.press(Key::Escape);
    }

    fn shift_down_does_not_walk_the_visual_range(&self) {
        self.press(Key::Home);
        self.press(Key::v);
        self.wait_icon_shows("a.txt");
        self.press(self.next);
        self.wait_icon_shows("b.txt");
        assert!(self.fixture.press(Key::Down, SHIFT));
        assert_eq!(
            self.visual_text().as_deref(),
            Some("VISUAL"),
            "{:?}",
            self.mode
        );
        self.assert_cursor_fill(
            "b.txt",
            &["a.txt", "b.txt"],
            " Shift+Down does not walk the visual range",
        );
        self.press(Key::Escape);
        self.press(Key::Escape);
    }

    fn run(&self) {
        self.fill_a_and_end_on_e();
        self.v_walks_a_range_and_space_toggles();
        self.escape_ends_the_range_and_keeps_the_fill();
        self.shift_v_subtracts_the_walked_span();
        self.a_new_v_anchors_at_the_cursor();
        self.shift_arrows_never_select();
        self.shift_down_does_not_walk_the_visual_range();
    }
}

#[test]
fn tenxer_visual_ranges_select_unset_and_keep_the_fill() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_fill::tenxer_visual_ranges_select_unset_and_keep_the_fill",
        || {
            let fixture = KeyboardFixture::new();
            let browser = tenxer_without_grouping(&fixture);
            for name in ["d.txt", "e.txt"] {
                std::fs::write(fixture._directory.path().join(name), b"preview").expect("file");
            }
            fixture.view.refresh();
            wait_until(|| entry_count(&browser) == 5);
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                let (next, previous) = if mode == BrowserMode::Icons {
                    (Key::Right, Key::Left)
                } else {
                    (Key::j, Key::k)
                };
                VisualWalk {
                    fixture: &fixture,
                    browser: browser.clone(),
                    mode,
                    next,
                    previous,
                }
                .run();
            }

            add_and_enter_empty_list_folder(&fixture, &browser);
            move_to_named(&fixture, &browser, "empty");
            fixture.press(Key::v, NONE);
            fixture.press(Key::Return, NONE);
            assert_eq!(
                fixture.shortcuts.visual_text(),
                None,
                "navigation ends the range"
            );
            wait_loaded(&browser, 0);
            assert_eq!(entry_count(&browser), 0);
            focus_files(&fixture);
            fixture.press(Key::v, NONE);
            assert_eq!(fixture.shortcuts.feedback_text(), "Nothing to select");
            assert_eq!(fixture.shortcuts.visual_text(), None);
        },
    );
}
