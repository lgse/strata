// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const CONTROL: ModifierType = ModifierType::CONTROL_MASK;
const SHIFT: ModifierType = ModifierType::SHIFT_MASK;

fn through(last: usize) -> Vec<usize> {
    (0..=last).collect()
}

fn shift_page_keys_extend_from_the_anchor(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    mode: BrowserMode,
) {
    assert!(fixture.press(Key::Page_Down, SHIFT), "{mode:?}");
    let paged = focused_index(browser);
    assert!(paged > 0, "{mode:?} Shift+Page Down moves the cursor");
    assert_eq!(
        fixture.selected(),
        through(paged),
        "{mode:?} Shift+Page Down extends from the anchor"
    );
    assert!(fixture.press(Key::Page_Up, SHIFT), "{mode:?}");
    assert_eq!(fixture.selected(), [0], "{mode:?} Shift+Page Up contracts");
}

fn plain_page_keys_replace_and_page(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    mode: BrowserMode,
) {
    assert!(fixture.press(Key::Page_Down, SHIFT), "{mode:?}");
    assert!(fixture.press(Key::Page_Down, NONE), "{mode:?}");
    assert_eq!(
        fixture.selected(),
        [focused_index(browser)],
        "{mode:?} a plain Page Down still replaces the selection"
    );

    let paged = focused_index(browser);
    assert!(fixture.press(Key::KP_Page_Up, NONE), "{mode:?}");
    let back = focused_index(browser);
    assert!(back < paged, "{mode:?} keypad Page Up pages");
    assert!(fixture.press(Key::KP_Page_Down, NONE), "{mode:?}");
    assert!(
        focused_index(browser) > back,
        "{mode:?} keypad Page Down pages"
    );
}

fn tenxer_shift_page_keys_do_not_select(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    mode: BrowserMode,
) {
    for key in [Key::Page_Down, Key::Page_Up] {
        assert!(fixture.press(key, SHIFT), "{mode:?} Shift+{key:?}");
        assert_eq!(
            (focused_index(browser), fixture.selected()),
            (0, vec![0]),
            "{mode:?} Shift+{key:?} does not move or select"
        );
    }
}

fn tenxer_page_keys_walk_the_visual_range(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    mode: BrowserMode,
) {
    fixture.press(Key::v, NONE);
    assert!(fixture.press(Key::Page_Down, NONE), "{mode:?}");
    let paged = focused_index(browser);
    assert!(paged > 0, "{mode:?} Page Down moves the cursor");
    assert_eq!(
        fixture.selected(),
        through(paged),
        "{mode:?} Page Down extends the visual range"
    );
    assert!(fixture.press(Key::Page_Up, NONE), "{mode:?}");
    let back = focused_index(browser);
    assert!(back < paged, "{mode:?}");
    assert_eq!(
        fixture.selected(),
        through(back),
        "{mode:?} Page Up shrinks the visual range"
    );
    fixture.press(Key::Escape, NONE);
    fixture.press(Key::Escape, NONE);
}

#[test]
fn page_keys_extend_shift_selection_or_a_tenxer_range() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::page_keys_extend_shift_selection_or_a_tenxer_range",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_group_by_type(false);
            let browser = fixture.view.browser();
            for index in 0..40 {
                std::fs::write(
                    fixture._directory.path().join(format!("n{index:02}.txt")),
                    b"n",
                )
                .expect("page file");
            }
            fixture.view.refresh();
            wait_until(|| entry_count(&browser) == 43);
            let modes = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons];
            for mode in modes {
                fixture.view.set_view_mode(mode);
                browser.select(0, 0);
                focus_files(&fixture);
                shift_page_keys_extend_from_the_anchor(&fixture, &browser, mode);
                plain_page_keys_replace_and_page(&fixture, &browser, mode);
            }

            PreferenceManager::shared().set_tenxer_mode(true);
            for mode in modes {
                fixture.view.set_view_mode(mode);
                browser.select(0, 0);
                focus_files(&fixture);
                pump(200);
                tenxer_shift_page_keys_do_not_select(&fixture, &browser, mode);
                tenxer_page_keys_walk_the_visual_range(&fixture, &browser, mode);
            }
        },
    );
}

/// Opens `folder` holding `folders` and waits for the cursor on the first one.
fn open_folders(fixture: &KeyboardFixture, folder: &std::path::Path, folders: &[&str]) {
    for name in folders {
        std::fs::create_dir_all(folder.join(name)).expect("fixture folder");
    }
    let browser = fixture.view.browser();
    browser.navigate(Location::local(folder));
    wait_loaded(&browser, 0);
    wait_until(|| focused_name(&browser) == folders[0]);
    focus_files(fixture);
}

#[test]
fn tenxer_columns_mirror_the_first_move_after_a_load() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::tenxer_columns_mirror_the_first_move_after_a_load",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_columns_mirror_selection(true);
            let browser = fixture.view.browser();
            let folder = fixture._directory.path().join("folders");
            open_folders(&fixture, &folder, &["a-dir", "b-dir"]);
            fixture.press(Key::j, NONE);
            assert_eq!(focused_name(&browser), "b-dir");
            wait_loaded(&browser, 1);
            assert!(location_ends_with(browser.location_at(1), "b-dir"));
            assert_eq!(focused_depth(&browser), Some(0));
        },
    );
}

#[test]
fn columns_mirror_does_not_cancel_an_open_that_is_still_validating() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::columns_mirror_does_not_cancel_an_open_that_is_still_validating",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_columns_mirror_selection(true);
            fixture.view.set_columns_mirror_selection(true);
            let browser = fixture.view.browser();
            let folder = fixture._directory.path().join("opening");
            std::fs::create_dir_all(folder.join("a-dir")).expect("fixture folder");
            std::fs::write(folder.join("a-dir/inner.txt"), b"inner").expect("fixture file");
            open_folders(&fixture, &folder, &["a-dir"]);

            fixture.press(Key::Return, NONE);
            assert!(browser.pending_navigation_generation().is_some());
            fixture.view.mirror_focused_folder_now();
            wait_until(|| focused_depth(&browser) == Some(1));
            assert!(location_ends_with(browser.location_at(1), "a-dir"));
            assert_eq!(focused_name(&browser), "inner.txt");
        },
    );
}

fn wait_mirrored(browser: &crate::app::Browser, name: &str) {
    wait_until(|| location_ends_with(browser.location_at(1), name));
    wait_loaded(browser, 1);
}

fn end_key_jumps_to_the_file_and_drops_the_mirror(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    end_key: Key,
) {
    fixture.press(Key::Down, NONE);
    assert_eq!(focused_name(browser), "b-dir", "{end_key:?}");
    wait_mirrored(browser, "b-dir");

    assert!(fixture.press(end_key, NONE), "Strata handles {end_key:?}");
    assert_cursor_fill(browser, "e.bin", &["e.bin"], format!("{end_key:?}"));
    wait_until(|| browser.location_at(1).is_none());
    assert_eq!(
        (fixture.view.item_view_has_focus(), focused_depth(browser)),
        (true, Some(0)),
        "{end_key:?}: (files focused, cursor depth)"
    );
}

fn home_key_jumps_to_the_first_folder_and_mirrors_it(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    end_key: Key,
    home_key: Key,
) {
    fixture.press(Key::Up, NONE);
    assert_eq!(focused_name(browser), "c-dir", "{end_key:?}");
    wait_mirrored(browser, "c-dir");

    assert!(fixture.press(home_key, NONE), "Strata handles {home_key:?}");
    assert_eq!(focused_name(browser), "a-dir", "{home_key:?}");
    wait_mirrored(browser, "a-dir");
}

#[test]
fn columns_mirror_home_and_end_in_the_default_key_map() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::columns_mirror_home_and_end_in_the_default_key_map",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            preferences.set_columns_mirror_selection(true);
            preferences.set_single_click_previews(false);
            fixture.view.set_columns_mirror_selection(true);
            fixture.view.set_single_click_previews(false);
            let browser = fixture.view.browser();
            let folder = fixture._directory.path().join("mixed");
            std::fs::create_dir_all(&folder).expect("fixture folder");
            std::fs::write(folder.join("e.bin"), b"e").expect("fixture file");
            open_folders(&fixture, &folder, &["a-dir", "b-dir", "c-dir"]);
            for (end_key, home_key) in [(Key::End, Key::Home), (Key::KP_End, Key::KP_Home)] {
                end_key_jumps_to_the_file_and_drops_the_mirror(&fixture, &browser, end_key);
                home_key_jumps_to_the_first_folder_and_mirrors_it(
                    &fixture, &browser, end_key, home_key,
                );
            }

            fixture.press(Key::Down, NONE);
            wait_mirrored(&browser, "b-dir");
            fixture.view.record_pointer_hover((12.0, 24.0), Some(0));
            assert!(!fixture.view.widget().has_css_class("keyboard-navigation"));
            assert!(fixture.press(Key::Down, CONTROL));
            assert_eq!(focused_name(&browser), "e.bin");
            wait_until(|| browser.location_at(1).is_none());
        },
    );
}

#[test]
fn home_and_end_jump_to_the_first_and_last_item_in_the_default_key_map() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::home_and_end_jump_to_the_first_and_last_item_in_the_default_key_map",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_group_by_type(false);
            let browser = fixture.view.browser();
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                fixture.view.set_view_mode(mode);
                focus_files(&fixture);
                browser.select(0, 0);
                browser.extend_selection(1);
                focus_files(&fixture);
                assert_cursor_fill(&browser, "b.txt", &["a.txt", "b.txt"], format!("{mode:?}"));

                assert!(fixture.press(Key::End, NONE), "{mode:?} Strata handles End");
                assert_cursor_fill(&browser, "c.txt", &["c.txt"], format!("{mode:?}"));
                assert!(fixture.view.item_view_has_focus(), "{mode:?}");
                if mode == BrowserMode::Columns {
                    wait_until(|| focused_widget_shows(&fixture.window, "c.txt"));
                }

                assert!(
                    fixture.press(Key::Home, NONE),
                    "{mode:?} Strata handles Home"
                );
                assert_cursor_fill(&browser, "a.txt", &["a.txt"], format!("{mode:?}"));
            }
        },
    );
}

/// A real press reaches the dispatcher first and the focused list's own GTK binding
/// only when the dispatcher lets it through; `KeyboardFixture::press` emits just the
/// former. Returns whether the dispatcher handled it.
fn press_through(fixture: &KeyboardFixture, key: Key, modifiers: ModifierType) -> bool {
    if fixture.press(key, modifiers) {
        return true;
    }
    assert!(
        crate::ui::focus_navigation::activate_native_binding(&fixture.window, key, modifiers),
        "the focused file list binds {key:?} with {modifiers:?}"
    );
    pump(50);
    false
}

/// One view's keys for the range-anchor walk. Columns extends ranges itself; List and
/// Icons leave Shift+arrows to GTK, whose arrow binding the dispatcher runs for an
/// arrow or its letter.
struct AnchorCase {
    mode: BrowserMode,
    next: Key,
    previous: Key,
    strata_extends: bool,
    plain_next: &'static [Key],
}

const ANCHOR_CASES: [AnchorCase; 3] = [
    AnchorCase {
        mode: BrowserMode::Columns,
        next: Key::Down,
        previous: Key::Up,
        strata_extends: true,
        plain_next: &[Key::Down],
    },
    AnchorCase {
        mode: BrowserMode::List,
        next: Key::Down,
        previous: Key::Up,
        strata_extends: false,
        plain_next: &[Key::Down, Key::j],
    },
    AnchorCase {
        mode: BrowserMode::Icons,
        next: Key::Right,
        previous: Key::Left,
        strata_extends: false,
        plain_next: &[Key::Right, Key::l],
    },
];

#[test]
fn home_end_and_page_keys_move_the_shift_range_anchor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::home_end_and_page_keys_move_the_shift_range_anchor",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_group_by_type(false);
            let browser = fixture.view.browser();
            // h j k l read as arrows only while type-to-search is off.
            PreferenceManager::shared().set_type_to_search(false);
            for case in &ANCHOR_CASES {
                fixture.view.set_view_mode(case.mode);
                focus_files(&fixture);
                browser.select(0, 0);
                focus_files(&fixture);
                home_and_end_set_the_anchor(&fixture, &browser, case);
                page_keys_set_the_anchor(&fixture, &browser, case);
                arrows_set_the_anchor(&fixture, &browser, case);
            }
        },
    );
}

fn home_and_end_set_the_anchor(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    case: &AnchorCase,
) {
    let mode = case.mode;
    press_through(fixture, case.next, NONE);
    assert_eq!(fill_names(browser), ["b.txt"], "{mode:?} next item");
    assert!(fixture.press(Key::End, NONE), "{mode:?} Strata handles End");
    assert_eq!(fill_names(browser), ["c.txt"], "{mode:?} End");
    wait_until(|| focused_widget_shows(&fixture.window, "c.txt"));
    assert_eq!(
        press_through(fixture, Key::Home, SHIFT),
        case.strata_extends,
        "{mode:?} Shift+Home routing"
    );
    assert_eq!(
        fill_names(browser),
        ["a.txt", "b.txt", "c.txt"],
        "{mode:?} Shift+Home after End extends from the last item"
    );

    assert!(
        fixture.press(Key::Home, NONE),
        "{mode:?} Strata handles Home"
    );
    assert_eq!(fill_names(browser), ["a.txt"], "{mode:?} Home");
    wait_until(|| focused_widget_shows(&fixture.window, "a.txt"));
    press_through(fixture, case.next, SHIFT);
    assert_eq!(
        fill_names(browser),
        ["a.txt", "b.txt"],
        "{mode:?} Shift+next after Home extends from the first item"
    );
    assert_eq!(
        press_through(fixture, Key::End, SHIFT),
        case.strata_extends,
        "{mode:?} Shift+End routing"
    );
    assert_eq!(
        fill_names(browser),
        ["a.txt", "b.txt", "c.txt"],
        "{mode:?} Shift+End keeps the anchor Home set"
    );
}

fn page_keys_set_the_anchor(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    case: &AnchorCase,
) {
    let mode = case.mode;
    assert!(fixture.press(Key::Home, NONE), "{mode:?}");
    assert!(fixture.press(Key::Page_Down, NONE), "{mode:?} Page Down");
    assert_eq!(fill_names(browser), ["c.txt"], "{mode:?} Page Down");
    wait_until(|| focused_widget_shows(&fixture.window, "c.txt"));
    press_through(fixture, case.previous, SHIFT);
    assert_eq!(
        fill_names(browser),
        ["b.txt", "c.txt"],
        "{mode:?} Shift+previous after Page Down extends from the paged item"
    );
}

fn arrows_set_the_anchor(
    fixture: &KeyboardFixture,
    browser: &crate::app::Browser,
    case: &AnchorCase,
) {
    let mode = case.mode;
    for &key in case.plain_next {
        assert!(fixture.press(Key::Home, NONE), "{mode:?}");
        wait_until(|| focused_widget_shows(&fixture.window, "a.txt"));
        press_through(fixture, key, NONE);
        assert_eq!(fill_names(browser), ["b.txt"], "{mode:?} {key:?}");
        wait_until(|| focused_widget_shows(&fixture.window, "b.txt"));
        assert!(
            fixture.press(Key::Page_Down, SHIFT),
            "{mode:?} Strata handles Shift+Page Down"
        );
        assert_eq!(
            fill_names(browser),
            ["b.txt", "c.txt"],
            "{mode:?} Shift+Page Down after {key:?} extends from the moved-to item"
        );
    }
}

#[test]
fn jump_keys_move_the_cursor_when_focus_is_parked_on_the_column_list() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::jump_keys::jump_keys_move_the_cursor_when_focus_is_parked_on_the_column_list",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_group_by_type(false);
            let browser = fixture.view.browser();
            for (forward, back) in [
                ((Key::End, NONE), (Key::Home, NONE)),
                ((Key::Down, CONTROL), (Key::Up, CONTROL)),
            ] {
                browser.select(0, 1);
                park_focus_on_the_column(&fixture);

                assert!(
                    fixture.press(forward.0, forward.1),
                    "{forward:?} while parked"
                );
                assert_eq!(focused_name(&browser), "c.txt", "{forward:?}");

                park_focus_on_the_column(&fixture);
                assert!(fixture.press(back.0, back.1), "{back:?} while parked");
                assert_eq!(focused_name(&browser), "a.txt", "{back:?}");
            }
            for (key, modifiers) in [
                (Key::Left, CONTROL),
                (Key::Up, NONE),
                (Key::Up, CONTROL | SHIFT),
                (Key::Up, CONTROL | ModifierType::ALT_MASK),
                (Key::Up, CONTROL | ModifierType::SUPER_MASK),
            ] {
                browser.select(0, 2);
                park_focus_on_the_column(&fixture);
                fixture.press(key, modifiers);
                assert_ne!(
                    focused_name(&browser),
                    "a.txt",
                    "{modifiers:?}+{key:?} must not jump"
                );
            }
        },
    );
}

fn park_focus_on_the_column(fixture: &KeyboardFixture) {
    focus_files(fixture);
    let focused = gtk::prelude::RootExt::focus(&fixture.window).expect("focused row");
    let list = if focused.is::<gtk::ListView>() {
        focused
    } else {
        focused
            .ancestor(gtk::ListView::static_type())
            .expect("column list")
    };
    // The column surface takes focus only off its content page, so park on the list
    // widget itself, as GTK does when the list has no focused row to restore.
    gtk::prelude::RootExt::set_focus(&fixture.window, Some(&list));
    assert_eq!(gtk::prelude::RootExt::focus(&fixture.window), Some(list));
    assert!(fixture.view.item_view_has_focus());
}
