// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
enum Refusal {
    OnlyChildDeleted,
    ParentDeleted,
    Unreadable,
    TimedOut,
}

#[derive(Clone, Copy, Debug)]
enum Cancellation {
    Escape,
    Delete,
    Selection,
    Chord,
}

fn save_target(path: &std::path::Path) {
    glib::MainContext::default()
        .block_on(crate::services::letter_bookmarks::set(
            'a',
            path.to_path_buf(),
        ))
        .expect("save target");
}

fn refused_jump(name: &str, refusal: Refusal) {
    crate::test_support::gtk_test(
        &format!(
            "ui::window::tests::keyboard_dispatch::letter_bookmarks::pending_jump::{name}"
        ),
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let tree = tempfile::tempdir().expect("target tree");
            let parent = tree.path().join("parent");
            let target = parent.join("target");
            std::fs::create_dir_all(&target).expect("only child folder");
            save_target(&target);
            let gate = crate::test_support::Gate::default();
            let mut quirks = crate::adapters::Quirks::default();
            match refusal {
                Refusal::OnlyChildDeleted => {
                    std::fs::remove_dir(&target).expect("delete only child")
                }
                Refusal::ParentDeleted => std::fs::remove_dir_all(&parent).expect("delete parent"),
                Refusal::Unreadable => quirks.fail.push((
                    crate::adapters::FsOp::OpenPath,
                    "*",
                    0,
                    rustix::io::Errno::ACCESS,
                )),
                Refusal::TimedOut => quirks.before.push((
                    crate::adapters::FsOp::FolderStamp,
                    "target",
                    0,
                    gate.racer(),
                )),
            }
            let _scope = crate::adapters::FsQuirks::install(tree.path(), quirks);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let selection = fixture.selected();
            let generation = browser.navigation_generation();
            mark(&fixture, Key::apostrophe, Key::a);
            assert_eq!(
                browser.navigation_generation(),
                generation,
                "checking is not navigation"
            );
            if matches!(refusal, Refusal::TimedOut) {
                wait_until(|| gate.reached());
            }
            wait_until_or(
                || fixture.shortcuts.feedback_text() == "Bookmark a is unavailable",
                || {
                    format!(
                        "feedback: {}; location: {:?}",
                        fixture.shortcuts.feedback_text(),
                        browser.active_location()
                    )
                },
            );
            gate.release();
            assert_eq!(browser.active_location(), origin);
            assert_eq!(fixture.selected(), selection);
            assert_eq!(browser.navigation_generation(), generation);
            assert_eq!(fixture.shortcuts.armed_chord(), None);
        },
    );
}

#[test]
fn letter_bookmark_only_child_deletion_stays_put() {
    refused_jump(
        "letter_bookmark_only_child_deletion_stays_put",
        Refusal::OnlyChildDeleted,
    );
}

#[test]
fn letter_bookmark_parent_deletion_stays_put() {
    refused_jump(
        "letter_bookmark_parent_deletion_stays_put",
        Refusal::ParentDeleted,
    );
}

#[test]
fn letter_bookmark_unreadable_folder_stays_put() {
    refused_jump(
        "letter_bookmark_unreadable_folder_stays_put",
        Refusal::Unreadable,
    );
}

#[test]
fn letter_bookmark_probe_timeout_stays_put() {
    refused_jump("letter_bookmark_probe_timeout_stays_put", Refusal::TimedOut);
}

#[test]
fn pending_letter_bookmark_cancel_preserves_selection_and_preview() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::pending_jump::pending_letter_bookmark_cancel_preserves_selection_and_preview",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            fixture.press(Key::i, ModifierType::empty());
            wait_until(|| fixture.preview.is_open() && fixture.view.item_view_has_focus());
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            for action in [
                Cancellation::Escape,
                Cancellation::Delete,
                Cancellation::Selection,
                Cancellation::Chord,
            ] {
                browser.select(0, 0);
                focus_files(&fixture);
                let tree = tempfile::tempdir().expect("target tree");
                let target = tree.path().join("target");
                std::fs::create_dir(&target).expect("target folder");
                save_target(&target);
                let gate = crate::test_support::Gate::default();
                let _scope = crate::adapters::FsQuirks::install(
                    tree.path(),
                    crate::adapters::Quirks {
                        before: vec![(
                            crate::adapters::FsOp::FolderStamp,
                            "target",
                            0,
                            gate.racer(),
                        )],
                        ..crate::adapters::Quirks::default()
                    },
                );
                let selection = fixture.selected();
                mark(&fixture, Key::apostrophe, Key::a);
                wait_until(|| gate.reached());
                assert_eq!(
                    fixture.shortcuts.armed_chord(),
                    Some(crate::ui::tenxer_mode::Chord::BookmarkGo)
                );
                match action {
                    Cancellation::Escape => {
                        assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    }
                    Cancellation::Delete => {
                        assert!(fixture.press(Key::d, ModifierType::empty()));
                    }
                    Cancellation::Selection => browser.select(0, 1),
                    Cancellation::Chord => fixture.shortcuts.cancel_chord(),
                }
                assert_eq!(
                    fixture.shortcuts.armed_chord(),
                    None,
                    "{action:?} cancels immediately"
                );
                let expected = if matches!(action, Cancellation::Selection) {
                    vec![1]
                } else {
                    selection
                };
                assert_eq!(
                    fixture.selected(),
                    expected,
                    "{action:?} must not dismiss selection"
                );
                assert!(
                    fixture.preview.is_open(),
                    "{action:?} must not dismiss the preview"
                );
                assert!(
                    !modal_visible(&fixture.overlay),
                    "pending d must not ask to delete"
                );
                gate.release();
                pump(100);
                assert_eq!(
                    browser.active_location(),
                    origin,
                    "{action:?} cancels navigation"
                );
                assert!(fixture._directory.path().join("a.txt").exists());
            }
        },
    );
}

#[test]
fn missing_letter_bookmark_does_not_cancel_navigation_validation() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::pending_jump::missing_letter_bookmark_does_not_cancel_navigation_validation",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let browser = fixture.view.browser();
            let home = glib::home_dir();
            let config = home.join(".config");
            std::fs::create_dir_all(&config).expect("config folder");
            let gate = crate::test_support::Gate::default();
            let _scope = crate::adapters::FsQuirks::install(
                &home,
                crate::adapters::Quirks {
                    before: vec![(
                        crate::adapters::FsOp::FolderStamp,
                        ".config",
                        0,
                        gate.racer(),
                    )],
                    ..crate::adapters::Quirks::default()
                },
            );
            focus_files(&fixture);
            fixture.press(Key::g, ModifierType::empty());
            fixture.press(Key::c, ModifierType::empty());
            wait_until(|| gate.reached());
            let generation = browser.navigation_generation();
            mark(&fixture, Key::apostrophe, Key::z);
            wait_until(|| fixture.shortcuts.feedback_text() == "No bookmark z");
            assert_eq!(browser.navigation_generation(), generation);
            gate.release();
            wait_until(|| browser.active_location() == Some(Location::local(&config)));
        },
    );
}

#[test]
fn letter_bookmark_read_errors_cancel_the_menu() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::pending_jump::letter_bookmark_read_errors_cancel_the_menu",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let state = glib::user_data_dir().join("strata/letter-bookmarks.json");
            std::fs::create_dir_all(state.parent().expect("parent")).expect("state directory");
            for corrupt in [true, false] {
                if corrupt {
                    std::fs::write(&state, "not JSON").expect("corrupt state");
                } else {
                    std::fs::remove_file(&state).expect("remove corrupt state");
                    std::fs::create_dir(&state).expect("unreadable state file");
                }
                fixture.shortcuts.dismiss_feedback();
                focus_files(&fixture);
                fixture.press(Key::apostrophe, ModifierType::empty());
                wait_until(|| {
                    fixture
                        .shortcuts
                        .feedback_text()
                        .starts_with("Could not access bookmarks:")
                });
                assert_eq!(fixture.shortcuts.armed_chord(), None);
            }
        },
    );
}

#[test]
fn nonlocal_letter_bookmark_reports_unavailable() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::letter_bookmarks::pending_jump::nonlocal_letter_bookmark_reports_unavailable",
        || {
            let fixture = KeyboardFixture::new();
            PreferenceManager::shared().set_tenxer_mode(true);
            let state = glib::user_data_dir().join("strata/letter-bookmarks.json");
            std::fs::create_dir_all(state.parent().expect("parent")).expect("state directory");
            std::fs::write(
                &state,
                br#"{"entries":{"a":"sftp://example.test/private"}}"#,
            )
            .expect("nonlocal mark");
            let origin = fixture.view.browser().active_location();
            mark(&fixture, Key::apostrophe, Key::a);
            wait_until(|| fixture.shortcuts.feedback_text() == "Bookmark a is unavailable");
            assert_eq!(fixture.view.browser().active_location(), origin);
        },
    );
}
