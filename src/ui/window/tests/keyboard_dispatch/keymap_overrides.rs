// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::shortcut_reference::{ReferenceScope, reference_sections};

/// Loads `default` overrides before any window or Settings page exists. The
/// fixture saves its own preferences afterwards, but the manager it uses was
/// already loaded from these.
fn fixture_with_saved(default: &[(&str, &[&str])]) -> (Rc<PreferenceManager>, KeyboardFixture) {
    PreferenceManager::seed_saved_keymap_for_test(default, &[]);
    let preferences = PreferenceManager::shared();
    let fixture = KeyboardFixture::new();
    (preferences, fixture)
}

fn reference_actions() -> Vec<&'static str> {
    reference_sections(ReferenceScope::window(BrowserMode::Columns))
        .into_iter()
        .flat_map(|section| section.rows)
        .map(|row| row.action)
        .collect()
}

#[test]
fn override_saved_before_settings_opens_changes_dispatch() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::override_saved_before_settings_opens_changes_dispatch",
        || {
            let (_preferences, fixture) = fixture_with_saved(&[("rename", &["<Control>e"])]);
            focus_files(&fixture);
            fixture.press(Key::F2, ModifierType::empty());
            pump(50);
            assert!(!fixture.view.rename_is_active(), "F2 is no longer bound");
            assert!(fixture.press(Key::e, ModifierType::CONTROL_MASK));
            wait_until(|| fixture.view.rename_is_active());
        },
    );
}

#[test]
fn empty_list_unbinds() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::empty_list_unbinds",
        || {
            let (_preferences, fixture) = fixture_with_saved(&[("duplicate", &[])]);
            assert!(!reference_actions().contains(&"Duplicate selected items"));
            fixture.view.set_view_mode(BrowserMode::List);
            focus_files(&fixture);
            let before = directory_names(fixture._directory.path());
            assert!(
                !fixture.press(Key::d, ModifierType::CONTROL_MASK),
                "Ctrl+D falls through to GTK"
            );
            pump(200);
            assert_eq!(directory_names(fixture._directory.path()), before);
        },
    );
}

#[test]
fn unbound_chord_step_leaves_the_chord_panel() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::unbound_chord_step_leaves_the_chord_panel",
        || {
            PreferenceManager::seed_saved_keymap_for_test(&[], &[("go.home", &[])]);
            let preferences = PreferenceManager::shared();
            let fixture = KeyboardFixture::new();
            preferences.set_tenxer_mode(true);
            fixture.shortcuts.bind_preferences(&preferences);
            focus_files(&fixture);
            fixture.press(Key::g, ModifierType::empty());
            let shown = |action: &str| {
                fixture
                    .shortcuts
                    .chord_options()
                    .is_some_and(|options| options.iter().any(|(_, shown)| shown == action))
            };
            wait_until(|| shown("Downloads"));
            assert!(!shown("Home"), "{:?}", fixture.shortcuts.chord_options());
        },
    );
}

/// A plain letter a user binds acts in the file list, but a focused text
/// field and type-to-search keep typing it.
#[test]
fn plain_printable_override_yields_to_typing() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::plain_printable_override_yields_to_typing",
        || {
            let (preferences, fixture) = fixture_with_saved(&[("rename", &["e"])]);
            preferences.set_type_to_search(false);
            focus_files(&fixture);
            assert!(fixture.press(Key::e, ModifierType::empty()));
            wait_until(|| fixture.view.rename_is_active());
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.view.rename_is_active());

            focus_files(&fixture);
            assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
            wait_until(|| fixture.view.filter_has_focus());
            assert!(!fixture.press(Key::e, ModifierType::empty()));
            pump(50);
            assert!(!fixture.view.rename_is_active(), "e types into the filter");
            assert!(fixture.press(Key::Escape, ModifierType::empty()));

            preferences.set_type_to_search(true);
            fixture.view.browser().select(0, 0);
            focus_files(&fixture);
            assert!(fixture.press(Key::e, ModifierType::empty()));
            wait_until(|| fixture.view.filter_has_focus());
            assert!(!fixture.view.rename_is_active(), "e starts type-to-search");
        },
    );
}

/// Shift reverses a sort step on the key it is bound to, not its shipped one,
/// unless that shifted key is another step's own.
#[test]
fn rebound_sort_step_reverses_with_shift() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::rebound_sort_step_reverses_with_shift",
        || {
            PreferenceManager::seed_saved_keymap_for_test(
                &[],
                &[
                    ("sort.name", &["b"]),
                    ("sort.size", &["a"]),
                    ("sort.type", &["<Shift>a"]),
                ],
            );
            let preferences = PreferenceManager::shared();
            let fixture = KeyboardFixture::new();
            preferences.set_tenxer_mode(true);
            fixture.shortcuts.bind_preferences(&preferences);
            let browser = fixture.view.browser();
            let sorting = || browser.column_preferences(0).expect("column").sort();
            let (shift, none) = (ModifierType::SHIFT_MASK, ModifierType::empty());
            let (name, size, modified) = (SortKey::Name, SortKey::Size, SortKey::Modified);
            let (up, down) = (SortDirection::Ascending, SortDirection::Descending);
            for (key, modifiers, expected) in [
                (Key::B, shift, (name, down)),
                (Key::b, none, (name, up)),
                (Key::A, shift, (SortKey::Type, up)),
                (Key::a, none, (size, up)),
                (Key::M, shift, (modified, down)),
            ] {
                focus_files(&fixture);
                assert!(fixture.press(Key::comma, ModifierType::empty()));
                assert!(fixture.press(key, modifiers));
                wait_until(|| sorting() == expected);
            }
        },
    );
}

/// A key a user moves `last_item` to jumps over Icons search hits as in List,
/// whatever modifier it holds.
#[test]
fn rebound_last_item_jumps_over_icon_search_hits() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::rebound_last_item_jumps_over_icon_search_hits",
        || {
            use super::footer_prompt::{
                ALL_REPORTS, commit_search, enable_tenxer, hit_cursor, seed_filter_tree,
                wait_results,
            };
            PreferenceManager::seed_saved_keymap_for_test(&[], &[("last_item", &["<Control>e"])]);
            let _preferences = PreferenceManager::shared();
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            enable_tenxer(&fixture).set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            for mode in [BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| column.count > 0)
                });
                commit_search(&fixture, "report");
                wait_results(&fixture, &ALL_REPORTS);
                let order = fixture.view.filter_result_names();
                wait_until(|| hit_cursor(&fixture).as_ref() == order.first());
                assert!(
                    fixture.press(Key::e, ModifierType::CONTROL_MASK),
                    "{mode:?}"
                );
                wait_until(|| hit_cursor(&fixture).as_ref() == order.last());
                for _ in 0..2 {
                    fixture.press(Key::Escape, ModifierType::empty());
                    pump(50);
                }
                wait_until(|| !fixture.view.listing_search_active());
            }
        },
    );
}

/// A typed key bound to the reference opens it in the default map where it
/// would not type, as 10xer's ~ does.
#[test]
fn typed_reference_key_toggles_the_reference() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::typed_reference_key_toggles_the_reference",
        || {
            let (preferences, fixture) =
                fixture_with_saved(&[("shortcut_reference", &["question"])]);
            preferences.set_tenxer_mode(false);
            preferences.set_type_to_search(false);
            focus_files(&fixture);
            assert!(fixture.press(Key::question, ModifierType::SHIFT_MASK));
            wait_until(|| fixture.shortcuts.reference_is_open());
        },
    );
}

/// A focused document resets its zoom on its own keys, wherever text size's
/// keys have moved.
#[test]
fn document_zoom_reset_follows_its_own_keys() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::document_zoom_reset_follows_its_own_keys",
        || {
            PreferenceManager::seed_saved_keymap_for_test(
                &[
                    ("text_reset", &["<Control>9"]),
                    ("pdf_zoom_reset", &["<Control>0", "<Control>e"]),
                ],
                &[],
            );
            let preferences = PreferenceManager::shared();
            let fixture = super::preview_ownership::ownership_fixture();
            preferences.set_tenxer_mode(false);
            let resets = Rc::new(Cell::new(0));
            let reset = gtk::gio::SimpleAction::new("reset-zoom", None);
            let counter = resets.clone();
            reset.connect_activate(move |_, _| counter.set(counter.get() + 1));
            let actions = gtk::gio::SimpleActionGroup::new();
            actions.add_action(&reset);
            fixture
                .preview
                .pane_widget()
                .insert_action_group("pdf", Some(&actions));
            select_named(&fixture, "long.txt");
            assert!(fixture.press(Key::space, ModifierType::empty()));
            let document = || {
                widget_with_class(&fixture.preview.widget(), "preview-virtual-list")
                    .filter(gtk::Widget::is_mapped)
            };
            wait_until(|| document().is_some());
            assert!(document().expect("document").grab_focus());
            let enlarged = preferences.text_size().stepped(1);
            preferences.set_text_size(enlarged);
            for (key, count) in [(Key::_0, 1), (Key::e, 2)] {
                assert!(fixture.press(key, ModifierType::CONTROL_MASK), "{key:?}");
                assert_eq!(resets.get(), count, "{key:?}");
            }
            assert_eq!(preferences.text_size(), enlarged);
            assert!(fixture.press(Key::_9, ModifierType::CONTROL_MASK));
            assert_eq!(resets.get(), 2);
            assert_eq!(
                preferences.text_size(),
                crate::ui::preferences::TextSize::default()
            );
        },
    );
}

/// A focused text preview is a document, so a plain letter a user adds to a
/// 10xer command reaches the listing from it as the shipped letter does.
#[test]
fn added_letter_leaves_a_text_preview_like_a_shipped_one() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_overrides::added_letter_leaves_a_text_preview_like_a_shipped_one",
        || {
            use crate::ui::keys::{Binding, Keymap, Map, bindings::RENAME, use_keymap};
            let fixture = super::preview_ownership::ownership_fixture();
            let browser = fixture.view.browser();
            let _keymap = use_keymap(Keymap::defaults().with(
                Map::Tenxer,
                RENAME,
                vec![Binding::plain(Key::r), Binding::plain(Key::e)],
            ));
            let text_focused = || {
                gtk::prelude::RootExt::focus(&fixture.window)
                    .is_some_and(|focused| focused.has_css_class("preview-text"))
            };
            for key in [Key::r, Key::e] {
                move_to_named(&fixture, &browser, "short.txt");
                fixture.press(Key::l, ModifierType::empty());
                wait_until(text_focused);
                assert!(fixture.press(key, ModifierType::empty()), "{key:?}");
                wait_until(|| {
                    fixture.shortcuts.open_prompt_kind()
                        == Some(crate::ui::tenxer_mode::Prompt::Rename)
                });
                fixture.shortcuts.dismiss_prompt();
            }
        },
    );
}
