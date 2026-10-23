// SPDX-License-Identifier: MIT

use super::*;

fn mark(fixture: &KeyboardFixture, prefix: Key, letter: Key) {
    focus_files(fixture);
    fixture.shortcuts.dismiss_feedback();
    assert!(fixture.press(prefix, ModifierType::empty()));
    assert!(fixture.shortcuts.armed_chord().is_some());
    assert!(fixture.press(letter, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.armed_chord(), None);
}

#[test]
fn letter_bookmarks_save_jump_and_replace_across_windows_and_views() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::letter_bookmarks_save_jump_and_replace_across_windows_and_views",
        || {
            let first = KeyboardFixture::new();
            let second = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            let first_location = first
                .view
                .browser()
                .active_location()
                .expect("first folder");
            let second_location = second
                .view
                .browser()
                .active_location()
                .expect("second folder");
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                first.view.set_view_mode(mode);
                second.view.set_view_mode(mode);
                first.view.browser().navigate(first_location.clone());
                second.view.browser().navigate(second_location.clone());
                wait_until(|| {
                    rendered_name(&first.view.widget(), "a.txt")
                        && rendered_name(&second.view.widget(), "a.txt")
                });
                mark(&first, Key::m, Key::a);
                wait_until(|| first.shortcuts.feedback_text() == "Saved bookmark a");
                mark(&second, Key::m, Key::b);
                wait_until(|| second.shortcuts.feedback_text() == "Saved bookmark b");
                focus_files(&second);
                second.press(Key::apostrophe, ModifierType::empty());
                let expected = vec![
                    (
                        "a".to_owned(),
                        first_location
                            .native_path()
                            .expect("local folder")
                            .display()
                            .to_string(),
                    ),
                    (
                        "b".to_owned(),
                        second_location
                            .native_path()
                            .expect("local folder")
                            .display()
                            .to_string(),
                    ),
                ];
                wait_until(|| second.shortcuts.chord_options() == Some(expected.clone()));
                second.press(Key::a, ModifierType::empty());
                wait_until(|| {
                    second.view.browser().active_location() == Some(first_location.clone())
                });
                wait_until(|| rendered_name(&second.view.widget(), "a.txt"));
                mark(&first, Key::apostrophe, Key::b);
                wait_until(|| {
                    first.view.browser().active_location() == Some(second_location.clone())
                });
                wait_until(|| rendered_name(&first.view.widget(), "a.txt"));
                mark(&first, Key::m, Key::a);
                wait_until(|| first.shortcuts.feedback_text() == "Saved bookmark a");
                mark(&second, Key::apostrophe, Key::a);
                wait_until(|| {
                    second.view.browser().active_location() == Some(second_location.clone())
                });
            }
        },
    );
}

#[test]
fn letter_bookmarks_menu_does_not_reopen_after_cancel_or_replace_the_save_menu() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::letter_bookmarks_menu_does_not_reopen_after_cancel_or_replace_the_save_menu",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            focus_files(&fixture);
            fixture.press(Key::apostrophe, ModifierType::empty());
            crate::storage::writer::settle();
            pump(20);
            assert_eq!(
                fixture.shortcuts.armed_chord(),
                Some(crate::ui::tenxer_mode::Chord::BookmarkGo)
            );
            assert_eq!(
                fixture.shortcuts.chord_options(),
                None,
                "no unsaved options"
            );
            fixture.press(Key::Escape, ModifierType::empty());
            mark(&fixture, Key::m, Key::a);
            wait_until(|| fixture.shortcuts.feedback_text() == "Saved bookmark a");
            for replace_with_save in [false, true] {
                let held = crate::storage::writer::hold();
                focus_files(&fixture);
                fixture.press(Key::apostrophe, ModifierType::empty());
                fixture.press(Key::Escape, ModifierType::empty());
                if replace_with_save {
                    fixture.press(Key::m, ModifierType::empty());
                }
                drop(held);
                crate::storage::writer::settle();
                pump(20);
                if replace_with_save {
                    wait_until(|| {
                        fixture
                            .shortcuts
                            .chord_options()
                            .is_some_and(|rows| rows.len() == 26)
                    });
                    assert_eq!(
                        fixture.shortcuts.armed_chord(),
                        Some(crate::ui::tenxer_mode::Chord::BookmarkSet)
                    );
                    fixture.press(Key::Escape, ModifierType::empty());
                } else {
                    assert_eq!(fixture.shortcuts.armed_chord(), None);
                    assert_eq!(fixture.shortcuts.chord_options(), None);
                }
            }
        },
    );
}

#[test]
fn letter_bookmarks_cancel_and_leave_default_mode_and_text_fields_unchanged() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::letter_bookmarks_cancel_and_leave_default_mode_and_text_fields_unchanged",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            focus_files(&fixture);
            for prefix in [Key::m, Key::apostrophe] {
                fixture.press(prefix, ModifierType::empty());
                assert_eq!(fixture.shortcuts.armed_chord(), None, "default mode");
            }
            preferences.set_tenxer_mode(true);
            for prefix in [Key::m, Key::apostrophe] {
                for (second, modifiers) in [
                    (Key::Escape, ModifierType::empty()),
                    (Key::_1, ModifierType::empty()),
                    (Key::a, ModifierType::CONTROL_MASK),
                    (Key::A, ModifierType::SHIFT_MASK),
                ] {
                    focus_files(&fixture);
                    fixture.press(prefix, ModifierType::empty());
                    assert!(fixture.shortcuts.armed_chord().is_some());
                    fixture.press(second, modifiers);
                    assert_eq!(fixture.shortcuts.armed_chord(), None);
                    if second != Key::Escape {
                        assert_eq!(fixture.shortcuts.feedback_text(), "Unknown chord");
                    }
                }
            }
            fixture.view.begin_location_edit();
            wait_until(|| fixture.view.location_has_focus());
            for prefix in [Key::m, Key::apostrophe] {
                fixture.press(prefix, ModifierType::empty());
                assert_eq!(fixture.shortcuts.armed_chord(), None, "location text input");
            }
            fixture.view.cancel_location_edit();
            mark(&fixture, Key::apostrophe, Key::z);
            wait_until(|| fixture.shortcuts.feedback_text() == "No bookmark z");
        },
    );
}

#[test]
fn letter_bookmarks_keep_the_listing_when_the_saved_directory_disappears() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::letter_bookmarks_keep_the_listing_when_the_saved_directory_disappears",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let saved = fixture._directory.path().to_path_buf();
            mark(&fixture, Key::m, Key::a);
            wait_until(|| fixture.shortcuts.feedback_text() == "Saved bookmark a");
            let other = tempfile::tempdir().expect("other folder");
            fixture
                .view
                .browser()
                .navigate(Location::local(other.path()));
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            std::fs::remove_dir_all(saved).expect("remove bookmarked folder");
            let origin = fixture.view.browser().active_location();
            mark(&fixture, Key::apostrophe, Key::a);
            wait_until(|| fixture.shortcuts.feedback_text() == "Bookmark a is unavailable");
            assert_eq!(fixture.view.browser().active_location(), origin);
        },
    );
}

#[test]
fn pending_letter_bookmark_does_not_navigate_after_cancel_or_mode_exit() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::pending_letter_bookmark_does_not_navigate_after_cancel_or_mode_exit",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(true);
            mark(&fixture, Key::m, Key::a);
            wait_until(|| fixture.shortcuts.feedback_text() == "Saved bookmark a");
            let other = tempfile::tempdir().expect("other folder");
            let origin = Location::local(other.path());
            fixture.view.browser().navigate(origin.clone());
            wait_until(|| {
                fixture
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            for exit_mode in [false, true] {
                let held = crate::storage::writer::hold();
                mark(&fixture, Key::apostrophe, Key::a);
                if exit_mode {
                    preferences.set_tenxer_mode(false);
                } else {
                    fixture.press(Key::Escape, ModifierType::empty());
                }
                drop(held);
                crate::storage::writer::settle();
                pump(50);
                assert_eq!(
                    fixture.view.browser().active_location(),
                    Some(origin.clone())
                );
            }
        },
    );
}

#[test]
fn letter_bookmark_cancel_during_folder_validation_keeps_the_current_directory() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::letter_bookmark_cancel_during_folder_validation_keeps_the_current_directory",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            open_empty_folder(&fixture);
            mark(&fixture, Key::m, Key::a);
            wait_until(|| fixture.shortcuts.feedback_text() == "Saved bookmark a");
            let browser = fixture.view.browser();
            let origin = Location::local(fixture._directory.path());
            browser.navigate(origin.clone());
            wait_loaded(&browser, 0);
            let gate = crate::test_support::Gate::default();
            let _scope = crate::adapters::FsQuirks::install(
                fixture._directory.path(),
                crate::adapters::Quirks {
                    before: vec![(crate::adapters::FsOp::FolderStamp, "empty", 0, gate.racer())],
                    ..crate::adapters::Quirks::default()
                },
            );
            mark(&fixture, Key::apostrophe, Key::a);
            wait_until(|| gate.reached());
            fixture.press(Key::Escape, ModifierType::empty());
            gate.release();
            pump(100);
            assert_eq!(browser.active_location(), Some(origin));
        },
    );
}
