// SPDX-License-Identifier: MIT

use super::*;

const NONE: ModifierType = ModifierType::empty();
const CONTROL: ModifierType = ModifierType::CONTROL_MASK;
const MODE_CHORD: ModifierType = ModifierType::CONTROL_MASK.union(ModifierType::SHIFT_MASK);

#[test]
fn appearance_menu_shows_i_preview_while_tenxer_is_on() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::entry_keys::appearance_menu_shows_i_preview_while_tenxer_is_on",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            let view = browser_for_window();
            let preview = PreviewDrawer::new(Rc::new(TextPreview), false);
            let menu = build_appearance_menu(&view, preferences.clone(), &preview);
            let window = gtk::Window::builder().child(&menu).build();
            window.present();
            menu.popup();
            wait_until(|| menu.popover().is_some_and(|popover| popover.is_visible()));
            let popover = menu.popover().expect("appearance popover");
            let toggle = widget_with_class(popover.upcast_ref(), "preview-panel-option")
                .expect("preview panel option");
            assert_eq!(preview_shortcut(&toggle), "Space");
            assert!(toggle.tooltip_text().is_none());
            preferences.set_tenxer_mode(true);
            assert_eq!(preview_shortcut(&toggle), "i");
            assert!(toggle.tooltip_text().is_none());
            preferences.set_tenxer_mode(false);
            assert_eq!(preview_shortcut(&toggle), "Space");
            window.destroy();
        },
    );
}

fn preview_shortcut(toggle: &gtk::Widget) -> String {
    widget_with_class(toggle, "folder-context-shortcut")
        .and_then(|widget| widget.downcast::<gtk::Label>().ok())
        .filter(|label| label.is_visible())
        .map(|label| label.text().to_string())
        .unwrap_or_default()
}

fn widget_text_visible(widget: &gtk::Widget, text: &str) -> bool {
    let shown_here = widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.is_visible() && label.text() == text);
    shown_here
        || std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
            .any(|child| widget_text_visible(&child, text))
}

fn reference_panel_visible(fixture: &KeyboardFixture) -> bool {
    widget_with_class(fixture.window.upcast_ref(), "shortcut-reference-panel")
        .is_some_and(|popover| popover.is_visible())
}

fn arm_go_chord_with_feedback(fixture: &KeyboardFixture) {
    let button = widget_with_class(fixture.window.upcast_ref(), "shortcut-footer-button")
        .expect("shortcuts button");
    assert_eq!(
        (button.is_visible(), fixture.shortcuts.tag_visible()),
        (false, true),
        "(shortcuts button shown, mode tag shown)"
    );
    fixture
        .shortcuts
        .arm_chord(crate::ui::tenxer_mode::Chord::Go);
    fixture.shortcuts.show_feedback("Copied");
    let chord = fixture.shortcuts.chord();
    assert_eq!(
        (chord.text().to_string(), chord.is_visible()),
        ("g-".to_owned(), true)
    );
}

fn the_prompt_cancels_the_chord(fixture: &KeyboardFixture) {
    assert!(
        fixture
            .shortcuts
            .open_prompt(crate::ui::tenxer_mode::Prompt::Find)
    );
    assert!(fixture.shortcuts.prompt().is_sensitive());
    assert_eq!(
        fixture.shortcuts.armed_chord(),
        None,
        "the prompt covers the chord mark, so it cancels the chord"
    );
}

fn the_prompt_keeps_its_text_through_delete_and_the_reference(fixture: &KeyboardFixture) {
    let prompt = fixture.shortcuts.prompt();
    prompt.set_text("keep");
    assert!(prompt.grab_focus());
    let names = directory_names(fixture._directory.path());
    let handled = fixture.press(Key::Delete, NONE);
    assert_eq!(
        (
            handled,
            prompt.text().to_string(),
            directory_names(fixture._directory.path())
        ),
        (false, "keep".to_owned(), names),
        "Delete in the prompt: (handled, prompt text, files)"
    );
    assert!(fixture.press(Key::F1, NONE));
    wait_until(|| reference_panel_visible(fixture));
    assert_eq!(prompt.text(), "keep");
    fixture.press(Key::Escape, NONE);
    wait_until(|| !reference_panel_visible(fixture));
    assert_eq!(prompt.text(), "keep");
}

fn escape_closes_the_prompt_and_feedback_dismisses(fixture: &KeyboardFixture) {
    let prompt = fixture.shortcuts.prompt();
    assert!(prompt.grab_focus());
    assert!(fixture.press(Key::Escape, NONE));
    assert_eq!(
        (fixture.shortcuts.prompt_label(), prompt.text().is_empty()),
        (None, true),
        "(prompt label, prompt cleared)"
    );
    fixture.shortcuts.dismiss_feedback();
    assert!(!widget_text_visible(
        fixture.shortcuts.widget().upcast_ref(),
        "Copied"
    ));
}

#[test]
fn hidden_shortcut_button_keeps_prompt_chord_and_feedback_usable() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::entry_keys::hidden_shortcut_button_keeps_prompt_chord_and_feedback_usable",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            preferences.set_show_keybinding_hints(false);
            fixture.shortcuts.bind_preferences(&preferences);
            pump(50);

            arm_go_chord_with_feedback(&fixture);
            the_prompt_cancels_the_chord(&fixture);
            the_prompt_keeps_its_text_through_delete_and_the_reference(&fixture);
            escape_closes_the_prompt_and_feedback_dismisses(&fixture);

            fixture
                .shortcuts
                .arm_chord(crate::ui::tenxer_mode::Chord::Go);
            assert!(fixture.shortcuts.chord().is_visible());
            preferences.set_tenxer_mode(false);
            pump(50);
            let chord = fixture.shortcuts.chord();
            assert_eq!(
                (
                    chord.is_visible(),
                    chord.text().is_empty(),
                    fixture.shortcuts.tag_visible()
                ),
                (false, true, false),
                "leaving 10xer: (chord shown, chord cleared, mode tag shown)"
            );
        },
    );
}

/// Shift and the key left of 1 type Ё on a Russian layout; only the
/// layout fallback reads it as ~.
fn russian_tilde(fixture: &KeyboardFixture) -> bool {
    fixture.keys.emit_by_name::<bool>(
        "key-pressed",
        &[&Key::Cyrillic_IO, &49u32, &ModifierType::SHIFT_MASK],
    )
}

/// (handled, 10xer on, selection, files)
fn letter_outcome(fixture: &KeyboardFixture, key: Key) -> (bool, bool, Vec<usize>, Vec<String>) {
    (
        fixture.press(key, NONE),
        PreferenceManager::shared().tenxer_mode(),
        fixture.selected(),
        directory_names(fixture._directory.path()),
    )
}

fn assert_selects_all(entry: &impl IsA<gtk::Editable>) {
    let entry = entry.as_ref();
    assert_eq!(
        entry.selection_bounds(),
        Some((0, entry.text().chars().count() as i32))
    );
}

fn an_inline_rename_types_the_letters(fixture: &KeyboardFixture, names: &[String]) {
    focus_files(fixture);
    // F2 renames in the footer here; the context menu still renames inline.
    assert!(fixture.view.begin_rename());
    let field = fixture.view.active_rename_field().expect("rename field");
    field.set_text("kept.txt");
    field.set_position(-1);
    for key in [Key::q, Key::d, Key::p, Key::a] {
        assert_eq!(
            letter_outcome(fixture, key),
            (false, true, vec![0], names.to_vec()),
            "{key:?}"
        );
        assert_eq!(field.text(), "kept.txt");
    }
    assert!(fixture.press(Key::a, CONTROL));
    assert_selects_all(&field);
    assert_eq!(fixture.selected(), [0]);
    assert!(fixture.press(Key::Escape, NONE));
}

fn the_location_types_the_letters_and_tildes(fixture: &KeyboardFixture, names: &[String]) {
    focus_files(fixture);
    assert!(fixture.press(Key::l, CONTROL));
    wait_until(|| fixture.view.location_has_focus());
    let location = focused_entry(&fixture.window);
    location.set_text("kept-location");
    location.set_position(-1);
    for key in [Key::q, Key::d, Key::p, Key::a] {
        assert_eq!(
            letter_outcome(fixture, key),
            (false, true, vec![0], names.to_vec()),
            "{key:?}"
        );
        assert_eq!(location.text(), "kept-location");
        assert!(fixture.view.location_has_focus());
    }
    assert!(
        !fixture.press(Key::asciitilde, NONE),
        "~ is typed into the location"
    );
    assert!(!russian_tilde(fixture), "Ё is typed into the location");
    assert_eq!(
        (
            fixture.shortcuts.reference_is_open(),
            fixture.view.location_has_focus()
        ),
        (false, true),
        "(reference open, location focused)"
    );
    assert!(fixture.press(Key::a, CONTROL));
    assert_selects_all(&location);
}

fn the_mode_chord_toggles_from_the_location(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    assert!(fixture.press(Key::m, MODE_CHORD));
    assert!(!preferences.tenxer_mode());
    assert!(fixture.view.location_has_focus());
    assert!(fixture.press(Key::m, MODE_CHORD));
    assert!(preferences.tenxer_mode());
    assert!(fixture.press(Key::Escape, NONE));
}

fn the_reference_search_keeps_its_keys(fixture: &KeyboardFixture, names: &[String]) {
    focus_files(fixture);
    assert!(fixture.press(Key::F1, NONE));
    wait_until(|| reference_panel_visible(fixture));
    assert!(
        !russian_tilde(fixture),
        "Ё is typed into the reference search"
    );
    assert!(fixture.shortcuts.reference_is_open());
    let (_, tenxer, selected, files) = letter_outcome(fixture, Key::q);
    assert_eq!(
        (tenxer, files, selected),
        (true, names.to_vec(), vec![0]),
        "q in the reference search"
    );
    fixture.press(Key::Delete, NONE);
    assert_eq!(
        directory_names(fixture._directory.path()),
        names,
        "Delete must not remove files while the reference is open"
    );
    assert!(fixture.press(Key::asciitilde, NONE));
    wait_until(|| !reference_panel_visible(fixture));
    focus_files(fixture);
    assert!(russian_tilde(fixture), "the file list reads Shift+ё as ~");
    wait_until(|| reference_panel_visible(fixture));
    assert!(fixture.press(Key::Escape, NONE));
    wait_until(|| !fixture.shortcuts.reference_is_open());
}

fn the_context_menu_keeps_its_keys(fixture: &KeyboardFixture, names: &[String]) {
    let window: &gtk::Widget = fixture.window.upcast_ref();
    focus_files(fixture);
    assert!(fixture.press(Key::Menu, NONE));
    wait_until(|| visible_menu(window).is_some());
    for key in [Key::q, Key::d, Key::a] {
        fixture.press(key, NONE);
    }
    assert_eq!(
        (
            PreferenceManager::shared().tenxer_mode(),
            directory_names(fixture._directory.path()),
            fixture.selected()
        ),
        (true, names.to_vec(), vec![0]),
        "q d a in the context menu: (10xer on, files, selection)"
    );
    visible_menu(window).expect("menu").popdown();
}

fn focused_modal_layer(overlay: &gtk::Overlay) -> gtk::Box {
    let modal = gtk::Box::new(gtk::Orientation::Vertical, 0);
    modal.set_focusable(true);
    modal.add_css_class("app-modal-layer");
    overlay.add_overlay(&modal);
    modal.grab_focus();
    modal
}

fn a_modal_layer_keeps_the_mode(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    let modal = focused_modal_layer(&fixture.overlay);
    wait_until(|| modal.has_focus());
    fixture.press(Key::m, MODE_CHORD);
    assert!(preferences.tenxer_mode());
    assert!(modal.is_visible());
    let capture = fixture.press(Key::comma, CONTROL);
    assert!(preferences.tenxer_mode());
    assert!(modal.has_focus() || capture);
}

fn press_settings(window: &gtk::ApplicationWindow, phase: gtk::PropagationPhase) -> bool {
    press_phase(window, phase, Key::comma, CONTROL)
}

fn settings_shown(content: &super::super::super::composition::WindowContent) -> bool {
    settings_layer(content).is_some_and(|layer| layer.is_visible())
}

fn ctrl_comma_reaches_settings_until_a_modal_blocks_it() {
    let preferences = PreferenceManager::shared();
    let (window, content) = composed_window();
    let directory = tempfile::tempdir().expect("settings folder");
    content
        .browser
        .navigate_location(Location::local(directory.path()));
    wait_loaded(&content.browser.browser(), 0);
    content.browser.browser().focus_active();
    wait_until(|| content.browser.item_view_has_focus());
    let capture = press_settings(&window, gtk::PropagationPhase::Capture);
    assert!(!capture, "Ctrl+, must reach Settings");
    assert!(press_settings(&window, gtk::PropagationPhase::Bubble));
    wait_until(|| settings_layer(&content).is_some());
    let layer = settings_layer(&content).expect("settings");
    assert!(layer.is_visible());
    layer.set_visible(false);
    let blocking = focused_modal_layer(content.overlay());
    press_settings(&window, gtk::PropagationPhase::Bubble);
    assert!(!settings_shown(&content));
    press_phase(&window, gtk::PropagationPhase::Capture, Key::m, MODE_CHORD);
    assert!(preferences.tenxer_mode());
    assert!(blocking.is_visible());
    window.destroy();
}

#[test]
fn tenxer_entries_menus_and_reference_keep_their_keys() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::entry_keys::tenxer_entries_menus_and_reference_keep_their_keys",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let names = directory_names(fixture._directory.path());

            an_inline_rename_types_the_letters(&fixture, &names);
            the_location_types_the_letters_and_tildes(&fixture, &names);
            the_mode_chord_toggles_from_the_location(&fixture);
            the_reference_search_keeps_its_keys(&fixture, &names);
            the_context_menu_keeps_its_keys(&fixture, &names);
            a_modal_layer_keeps_the_mode(&fixture);
            ctrl_comma_reaches_settings_until_a_modal_blocks_it();
        },
    );
}

fn press_code(fixture: &KeyboardFixture, key: Key, keycode: u32, modifiers: ModifierType) -> bool {
    fixture
        .keys
        .emit_by_name::<bool>("key-pressed", &[&key, &keycode, &modifiers])
}

fn clipboard_text(fixture: &KeyboardFixture) -> Option<String> {
    fixture
        .window
        .clipboard()
        .content()
        .and_then(|content| content.value(glib::Type::STRING).ok())
        .and_then(|value| value.get::<String>().ok())
}

/// A Russian-only keymap types Cyrillic on the US A, C, Z and Y keys.
fn cyrillic_select_all_and_copy(fixture: &KeyboardFixture, entry: &gtk::Entry) {
    assert!(
        !press_code(fixture, Key::c, 0, CONTROL),
        "GTK's own Ctrl+C handles ASCII"
    );
    assert!(press_code(fixture, Key::Cyrillic_ef, 38, CONTROL));
    assert_eq!(entry.selection_bounds(), Some((0, 5)));
    let copied = press_code(fixture, Key::Cyrillic_es, 54, CONTROL);
    assert_eq!(
        (
            copied,
            clipboard_text(fixture).as_deref(),
            fixture.selected()
        ),
        (true, Some("draft"), vec![0]),
        "Ctrl+С: (handled, clipboard, selection)"
    );
}

fn cyrillic_undo_and_redo(fixture: &KeyboardFixture, search: &gtk::Entry) {
    assert!(
        !press_code(fixture, Key::z, 0, CONTROL),
        "GTK's own Ctrl+Z handles ASCII"
    );
    for (key, keycode, modifiers, text) in [
        (Key::Cyrillic_ya, 52, CONTROL, Some("half")),
        (
            Key::Cyrillic_YA,
            52,
            CONTROL | ModifierType::SHIFT_MASK,
            Some("half a page"),
        ),
        (Key::Cyrillic_ya, 52, CONTROL, None),
        (Key::Cyrillic_en, 29, CONTROL, Some("half a page")),
    ] {
        assert!(press_code(fixture, key, keycode, modifiers), "{key:?}");
        if let Some(text) = text {
            assert_eq!(search.text(), text, "{key:?}");
        }
    }
}

#[test]
fn layout_fallback_editing_keys_run_the_focused_text_fields_own_actions() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::entry_keys::layout_fallback_editing_keys_run_the_focused_text_fields_own_actions",
        || {
            let fixture = KeyboardFixture::new();
            let entry = gtk::Entry::new();
            entry.set_text("draft");
            fixture.overlay.add_overlay(&entry);
            entry.grab_focus();
            wait_until(|| {
                gtk::prelude::RootExt::focus(&fixture.window)
                    .is_some_and(|focus| focus.is_ancestor(&entry))
            });
            cyrillic_select_all_and_copy(&fixture, &entry);

            assert!(fixture.press(Key::F1, NONE));
            wait_until(|| fixture.shortcuts.reference_is_open());
            let search = focused_entry(&fixture.window);
            assert_ne!(search, entry, "the reference search has focus");
            search.set_text("half");
            search.set_position(-1);
            // Typing makes an undoable edit; a programmatic insert does not.
            search
                .delegate()
                .expect("search text")
                .emit_by_name::<()>("insert-at-cursor", &[&" a page"]);
            cyrillic_undo_and_redo(&fixture, &search);
        },
    );
}
