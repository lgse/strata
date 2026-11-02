// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const CONTROL: ModifierType = ModifierType::CONTROL_MASK;
const SHIFT: ModifierType = ModifierType::SHIFT_MASK;
const ALT: ModifierType = ModifierType::ALT_MASK;

fn pressed(fixture: &KeyboardFixture, key: Key, modifiers: ModifierType) {
    assert!(fixture.press(key, modifiers), "{modifiers:?}+{key:?}");
}

fn counter() -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let count = Rc::new(Cell::new(0));
    (count.clone(), count)
}

fn counted_window_action(fixture: &KeyboardFixture, name: &str) -> Rc<Cell<usize>> {
    let (count, observed) = counter();
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| observed.set(observed.get() + 1));
    fixture.window.add_action(&action);
    count
}

struct Counters {
    jumps: Rc<Cell<usize>>,
    searches: Rc<Cell<usize>>,
    pins: Rc<Cell<usize>>,
}

impl Counters {
    fn install(fixture: &KeyboardFixture) -> Self {
        let jumps = counted_window_action(fixture, "jump-folder");
        let searches = counted_window_action(fixture, "search");
        let (pins, observed) = counter();
        fixture.view.set_pin_handlers(
            Rc::new(move |_location, _name, done| {
                observed.set(observed.get() + 1);
                done(true);
            }),
            Rc::new(|_location| {}),
            Rc::new(|_| PinStatus::Available),
        );
        Self {
            jumps,
            searches,
            pins,
        }
    }
}

fn fixture_names(fixture: &KeyboardFixture) -> Vec<String> {
    directory_names(fixture._directory.path())
}

fn select_first(fixture: &KeyboardFixture) {
    fixture.view.browser().select(0, 0);
    focus_files(fixture);
}

/// (selection, filter focused, preview open, renaming, 10xer on)
fn letter_effects(fixture: &KeyboardFixture) -> (Vec<usize>, bool, bool, bool, bool) {
    (
        fixture.selected(),
        fixture.view.filter_has_focus(),
        fixture.preview.is_open(),
        fixture.view.rename_is_active(),
        PreferenceManager::shared().tenxer_mode(),
    )
}

fn default_letters_stay_inert(fixture: &KeyboardFixture) {
    for key in [Key::s, Key::S, Key::y] {
        select_first(fixture);
        fixture.press(key, NONE);
        assert_eq!(
            letter_effects(fixture),
            (vec![0], false, false, false, true),
            "{key:?} must not move the selection, search, preview, rename or leave the mode"
        );
    }
    select_first(fixture);
    fixture.press(Key::space, NONE);
    assert_ne!(
        fixture.selected(),
        [0],
        "Space toggles the cursor instead of previewing"
    );
    assert!(!fixture.preview.is_open());
    assert!(!fixture.view.rename_is_active());
    for key in [Key::j, Key::k] {
        select_first(fixture);
        fixture.press(key, NONE);
        let (_, searching, previewing, renaming, tenxer) = letter_effects(fixture);
        assert_eq!(
            (searching, previewing, renaming, tenxer),
            (false, false, false, true),
            "{key:?} must not search, preview, rename or leave the mode"
        );
    }
}

fn p_does_not_pin(fixture: &KeyboardFixture, counters: &Counters) {
    select_named(fixture, "folder");
    fixture.press(Key::Y, SHIFT);
    fixture.shortcuts.dismiss_feedback();
    fixture.press(Key::p, NONE);
    assert_eq!(counters.pins.get(), 0, "p must not pin while 10xer is on");
    assert_eq!(fixture.shortcuts.feedback_text(), "Nothing to paste");
    fixture.shortcuts.dismiss_feedback();
}

fn ctrl_paging_and_backslash_stay_inert(fixture: &KeyboardFixture) {
    select_first(fixture);
    let names_before_paging = fixture_names(fixture);
    for key in [Key::d, Key::f, Key::b] {
        fixture.press(key, CONTROL);
        assert!(
            !fixture.view.filter_has_focus(),
            "{key:?} must not open the filter"
        );
        assert!(
            fixture.sidebar_toggle.is_active(),
            "{key:?} must not toggle the sidebar"
        );
    }
    assert_eq!(
        fixture_names(fixture),
        names_before_paging,
        "Ctrl+D must not duplicate while 10xer is on"
    );
    select_first(fixture);
    fixture.press(Key::r, CONTROL);
    assert_ne!(fixture.selected(), [0], "Ctrl+R inverts the pane");
    assert!(!fixture.view.rename_is_active());
    select_first(fixture);
    fixture.press(Key::backslash, CONTROL);
    assert_eq!(fixture.selected(), [0]);
    assert!(!fixture.view.rename_is_active());
}

/// (files, filter focused, sidebar shown, renaming, folder jumps, modal open,
/// arrows scoped, selection, 10xer on)
type InertState = (
    Vec<String>,
    bool,
    bool,
    bool,
    usize,
    bool,
    bool,
    Vec<usize>,
    bool,
);

fn inert_state(fixture: &KeyboardFixture, counters: &Counters) -> InertState {
    let preferences = PreferenceManager::shared();
    (
        fixture_names(fixture),
        fixture.view.filter_has_focus(),
        fixture.sidebar_toggle.is_active(),
        fixture.view.rename_is_active(),
        counters.jumps.get(),
        modal_visible(&fixture.overlay),
        preferences.arrow_navigation_scoped(),
        fixture.selected(),
        preferences.tenxer_mode(),
    )
}

fn ctrl_t_and_jump_chord_stay_inert(
    fixture: &KeyboardFixture,
    counters: &Counters,
    names: Vec<String>,
) {
    let children = child_commands();
    fixture.press(Key::t, CONTROL);
    pump(200);
    assert_eq!(
        child_commands(),
        children,
        "Ctrl+T must not launch a terminal"
    );
    fixture.press(Key::k, CONTROL | SHIFT);
    pump(400);
    assert_eq!(
        inert_state(fixture, counters),
        (names, false, true, false, 0, false, true, vec![0], true),
        "Ctrl+T must not open a terminal and Ctrl+Shift+K must not jump"
    );
}

fn text_size_and_clipboard_keys_work(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    let text_size = preferences.text_size().root_font_px();
    for (key, expected) in [(Key::plus, text_size + 1), (Key::_0, 13)] {
        pressed(fixture, key, CONTROL);
        assert_eq!(preferences.text_size().root_font_px(), expected, "{key:?}");
    }
    let directory = fixture._directory.path();
    select_named(fixture, "a.txt");
    pressed(fixture, Key::c, CONTROL);
    select_named(fixture, "folder");
    pressed(fixture, Key::v, CONTROL);
    wait_until(|| directory.join("folder/a.txt").exists());
    select_named(fixture, "a.txt");
    pressed(fixture, Key::x, CONTROL);
    assert!(directory.join("a.txt").exists());
    select_named(fixture, "b.txt");
    pressed(fixture, Key::Delete, SHIFT);
    wait_until(|| modal_visible(&fixture.overlay));
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
}

fn rename_reload_and_location_keys_work(fixture: &KeyboardFixture, counters: &Counters) {
    select_named(fixture, "a.txt");
    pressed(fixture, Key::F2, NONE);
    assert!(!fixture.view.rename_is_active(), "F2 renames in the footer");
    assert_eq!(
        fixture.shortcuts.open_prompt_kind(),
        Some(crate::ui::tenxer_mode::Prompt::Rename)
    );
    pressed(fixture, Key::Escape, NONE);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    std::fs::write(fixture._directory.path().join("d.txt"), b"d").expect("refresh file");
    pressed(fixture, Key::F5, NONE);
    wait_until(|| rendered_name(&fixture.view.widget(), "d.txt"));
    pressed(fixture, Key::l, CONTROL);
    wait_until(|| fixture.view.location_has_focus());
    pressed(fixture, Key::Escape, NONE);
    focus_files(fixture);
    pressed(fixture, Key::k, CONTROL);
    assert_eq!(counters.searches.get(), 1);
}

fn view_mode_and_creation_keys_work(fixture: &KeyboardFixture) {
    for (key, mode) in [
        (Key::_2, BrowserMode::Icons),
        (Key::_3, BrowserMode::List),
        (Key::_1, BrowserMode::Columns),
    ] {
        pressed(fixture, key, CONTROL);
        assert_eq!(fixture.view.view_mode(), mode);
    }
    focus_files(fixture);
    pressed(fixture, Key::n, CONTROL | SHIFT);
    assert!(fixture.view.new_entry_is_active());
    pressed(fixture, Key::Escape, NONE);
    assert!(!fixture.view.new_entry_is_active());
    focus_files(fixture);
    pressed(fixture, Key::Return, ALT);
    wait_until(|| widget_with_class(fixture.overlay.upcast_ref(), "properties-content").is_some());
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
}

fn menu_keys_open_the_context_menu(fixture: &KeyboardFixture) {
    let window: &gtk::Widget = fixture.window.upcast_ref();
    focus_files(fixture);
    pressed(fixture, Key::Menu, NONE);
    wait_until(|| visible_menu(window).is_some());
    visible_menu(window).expect("menu").popdown();
    wait_until(|| visible_menu(window).is_none());
    focus_files(fixture);
    pressed(fixture, Key::F10, SHIFT);
    wait_until(|| visible_menu(window).is_some());
    visible_menu(window).expect("menu").popdown();
}

fn modified_shift_q_keeps_the_window(fixture: &KeyboardFixture) {
    focus_files(fixture);
    let names_before_quit = fixture_names(fixture);
    for modifier in [CONTROL, ALT, ModifierType::SUPER_MASK] {
        fixture.press(Key::Q, modifier | SHIFT);
        assert!(
            fixture.window.is_visible(),
            "{modifier:?}+Shift+Q must not close the window"
        );
        assert!(PreferenceManager::shared().tenxer_mode());
    }
    assert_eq!(fixture_names(fixture), names_before_quit);
}

fn q_stays_and_the_mode_chord_leaves(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    pressed(fixture, Key::q, NONE);
    assert!(preferences.tenxer_mode(), "q does not leave the mode");
    assert!(
        !fixture.view.filter_has_focus(),
        "q does not type-to-search"
    );
    pressed(fixture, Key::m, CONTROL | SHIFT);
    assert!(!preferences.tenxer_mode());
    assert!(fixture.window.is_visible());
}

fn default_map_keys_return(fixture: &KeyboardFixture, counters: &Counters) {
    focus_files(fixture);
    pressed(fixture, Key::s, NONE);
    assert!(fixture.view.filter_has_focus(), "type-to-search returns");
    pressed(fixture, Key::Escape, NONE);
    focus_files(fixture);
    pressed(fixture, Key::f, CONTROL);
    assert!(fixture.view.filter_has_focus(), "Ctrl+F filters again");
    pressed(fixture, Key::Escape, NONE);
    focus_files(fixture);
    assert!(fixture.sidebar_toggle.is_active());
    pressed(fixture, Key::b, CONTROL);
    assert!(!fixture.sidebar_toggle.is_active());
    focus_files(fixture);
    let jumps_before = counters.jumps.get();
    pressed(fixture, Key::k, CONTROL | SHIFT);
    assert_eq!(counters.jumps.get(), jumps_before + 1);
}

fn home_row_moves_without_type_to_search(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    preferences.set_type_to_search(false);
    select_first(fixture);
    pressed(fixture, Key::j, NONE);
    assert_ne!(fixture.selected(), [0], "home row moves again");
    assert!(!fixture.press(Key::backslash, CONTROL));
    assert!(preferences.arrow_navigation_scoped());
}

fn shift_q_closes_only_this_window(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    preferences.set_tenxer_mode(true);
    let other = gtk::Window::new();
    other.present();
    pressed(fixture, Key::Q, SHIFT);
    assert_eq!(
        (
            fixture.window.is_visible(),
            other.is_visible(),
            preferences.tenxer_mode()
        ),
        (false, true, true),
        "Shift+Q: (this window shown, other window shown, 10xer on)"
    );
}

fn tenxer_with_a_folder() -> (KeyboardFixture, Counters) {
    let fixture = KeyboardFixture::new();
    let preferences = PreferenceManager::shared();
    pressed(&fixture, Key::r, CONTROL);
    assert!(fixture.view.rename_is_active());
    pressed(&fixture, Key::Escape, NONE);
    focus_files(&fixture);
    preferences.set_type_to_search(true);
    preferences.set_arrow_navigation_scoped(true);
    preferences.set_tenxer_mode(true);
    let counters = Counters::install(&fixture);
    std::fs::create_dir(fixture._directory.path().join("folder")).expect("folder");
    fixture.view.refresh();
    wait_until(|| {
        fixture
            .view
            .browser()
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
            && rendered_name(&fixture.view.widget(), "folder")
    });
    focus_files(&fixture);
    (fixture, counters)
}

#[test]
fn tenxer_file_list_skips_conflicting_defaults_and_keeps_bound_shortcuts() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_shortcuts::tenxer_file_list_skips_conflicting_defaults_and_keeps_bound_shortcuts",
        || {
            let (fixture, counters) = tenxer_with_a_folder();
            let names = fixture_names(&fixture);

            default_letters_stay_inert(&fixture);
            p_does_not_pin(&fixture, &counters);
            ctrl_paging_and_backslash_stay_inert(&fixture);
            ctrl_t_and_jump_chord_stay_inert(&fixture, &counters, names);

            text_size_and_clipboard_keys_work(&fixture);
            rename_reload_and_location_keys_work(&fixture, &counters);
            view_mode_and_creation_keys_work(&fixture);
            menu_keys_open_the_context_menu(&fixture);
            modified_shift_q_keeps_the_window(&fixture);
            q_stays_and_the_mode_chord_leaves(&fixture);
            default_map_keys_return(&fixture, &counters);
            home_row_moves_without_type_to_search(&fixture);
            shift_q_closes_only_this_window(&fixture);
        },
    );
}

/// A plain letter opens the filter instead, which Escape closes.
fn near_miss(fixture: &KeyboardFixture, key: Key, modifiers: ModifierType) {
    let browser = fixture.view.browser();
    browser.select(0, 0);
    focus_files(fixture);
    fixture.press(key, modifiers);
    if fixture.view.filter_has_focus() {
        pressed(fixture, Key::Escape, NONE);
    }
    wait_loaded(&browser, 0);
}

fn only_ctrl_r_renames(fixture: &KeyboardFixture) {
    for (key, modifiers) in [
        (Key::r, NONE),
        (Key::r, CONTROL | SHIFT),
        (Key::r, CONTROL | ALT),
        (Key::F2, CONTROL),
        (Key::F5, NONE),
        (Key::F5, CONTROL),
    ] {
        near_miss(fixture, key, modifiers);
        assert!(
            !fixture.view.rename_is_active(),
            "{modifiers:?}+{key:?} must not rename"
        );
        // F2 waits out a refresh's row rebuild, so each near miss meets
        // a pane that can rename.
        wait_until(|| fixture.press(Key::F2, NONE));
        assert!(fixture.view.rename_is_active());
        pressed(fixture, Key::Escape, NONE);
    }
    select_first(fixture);
    pressed(fixture, Key::r, CONTROL);
    assert!(fixture.view.rename_is_active(), "Ctrl+R renames");
    pressed(fixture, Key::Escape, NONE);
}

fn only_ctrl_c_copies(fixture: &KeyboardFixture) {
    use crate::ui::browser::{ClipboardMark, clipboard_mark};
    let first = Location::local(fixture._directory.path().join("a.txt"));
    select_first(fixture);
    fixture.press(Key::c, CONTROL | SHIFT);
    assert_eq!(clipboard_mark(&first), ClipboardMark::None, "Ctrl+Shift+C");
    pressed(fixture, Key::c, CONTROL);
    assert_eq!(clipboard_mark(&first), ClipboardMark::Copy);
}

fn only_ctrl_h_and_period_toggle_hidden_files(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    let shows_hidden = browser.preferences().show_hidden;
    for (key, modifiers) in [
        (Key::h, NONE),
        (Key::h, CONTROL | SHIFT),
        (Key::h, CONTROL | ALT),
        (Key::greater, CONTROL | SHIFT),
    ] {
        near_miss(fixture, key, modifiers);
        assert_eq!(
            browser.preferences().show_hidden,
            shows_hidden,
            "{modifiers:?}+{key:?} must not toggle hidden files"
        );
    }
    for key in [Key::h, Key::period] {
        focus_files(fixture);
        for shown in [!shows_hidden, shows_hidden] {
            assert!(fixture.press(key, CONTROL), "Ctrl+{key:?}");
            assert_eq!(browser.preferences().show_hidden, shown);
        }
    }
}

#[test]
fn default_commands_ignore_near_miss_chords() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tenxer_shortcuts::default_commands_ignore_near_miss_chords",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_type_to_search(true);

            only_ctrl_r_renames(&fixture);
            only_ctrl_c_copies(&fixture);
            only_ctrl_h_and_period_toggle_hidden_files(&fixture);

            focus_files(&fixture);
            pressed(&fixture, Key::b, CONTROL);
            assert!(
                !fixture.sidebar_toggle.is_active(),
                "Ctrl+B hides the sidebar"
            );
            pump(50);
            assert!(!sidebar_has_focus(&fixture), "Ctrl+B does not focus it");
            focus_files(&fixture);
            assert!(fixture.press(Key::B, CONTROL | SHIFT), "Ctrl+Shift+B");
            wait_until(|| sidebar_has_focus(&fixture));
        },
    );
}
