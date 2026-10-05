// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::*;
use crate::ui::{
    browser::{ClipboardMark, CreateRefusal, clipboard_mark},
    shortcut_footer::CandidateKeys,
    tenxer_mode::Prompt,
};

fn enable_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
    fixture.view.browser().clear_active_selection();
}

fn plain(fixture: &KeyboardFixture, key: Key) {
    assert!(fixture.press(key, ModifierType::empty()), "{key:?}");
}

fn shifted(fixture: &KeyboardFixture, key: Key) {
    assert!(
        fixture.press(key, ModifierType::SHIFT_MASK),
        "Shift+{key:?}"
    );
}

fn clipboard() -> gtk::gdk::Clipboard {
    gtk::gdk::Display::default().expect("display").clipboard()
}

fn clipboard_value(kind: glib::Type) -> Option<glib::Value> {
    clipboard().content()?.value(kind).ok()
}

fn clipboard_files() -> Vec<PathBuf> {
    clipboard_value(gtk::gdk::FileList::static_type())
        .and_then(|value| value.get::<gtk::gdk::FileList>().ok())
        .map(|files| {
            files
                .files()
                .iter()
                .filter_map(|file| file.path())
                .collect()
        })
        .unwrap_or_default()
}

fn clipboard_text() -> Option<String> {
    clipboard_value(glib::Type::STRING).and_then(|value| value.get::<String>().ok())
}

fn mark(directory: &Path, name: &str) -> ClipboardMark {
    clipboard_mark(&Location::local(directory.join(name)))
}

fn feedback(fixture: &KeyboardFixture) -> String {
    let text = fixture.shortcuts.feedback_text();
    fixture.shortcuts.dismiss_feedback();
    text
}

fn focused_button(fixture: &KeyboardFixture) -> Option<gtk::Button> {
    gtk::prelude::RootExt::focus(&fixture.window)?
        .downcast::<gtk::Button>()
        .ok()
}

fn wait_focused_button(fixture: &KeyboardFixture, label: &str) -> gtk::Button {
    wait_until(|| {
        focused_button(fixture).is_some_and(|button| button.label().as_deref() == Some(label))
    });
    focused_button(fixture).expect("focused button")
}

fn close_modal(fixture: &KeyboardFixture) {
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
}

/// Window dispatch cannot reach the modal's own key controller.
fn modal_key(fixture: &KeyboardFixture, key: Key) -> bool {
    let layer =
        widget_with_class(fixture.overlay.upcast_ref(), "app-modal-layer").expect("open dialog");
    let controllers = layer.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("dialog key controller");
    keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::empty()])
}

fn contents(path: &Path) -> String {
    std::fs::read_to_string(path).expect("fixture contents")
}

fn fill_b_and_c(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    browser.clear_active_selection();
    move_to_named(fixture, &browser, "b.txt");
    plain(fixture, Key::space);
    plain(fixture, Key::space);
    move_to_named(fixture, &browser, "a.txt");
    assert_eq!(fill_names(&browser), ["b.txt", "c.txt"]);
}

fn enter_folder(fixture: &KeyboardFixture, name: &str) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, name);
    plain(fixture, Key::l);
    wait_until(|| {
        browser.active_depth() == Some(1) && location_ends_with(browser.active_location(), name)
    });
    wait_loaded(&browser, 1);
    focus_files(fixture);
}

fn leave_folder(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    plain(fixture, Key::h);
    wait_until(|| browser.active_depth() == Some(0));
    focus_files(fixture);
}

fn open_empty_folder(fixture: &KeyboardFixture) {
    let empty = fixture._directory.path().join("empty");
    std::fs::create_dir(&empty).expect("empty folder");
    let browser = fixture.view.browser();
    browser.navigate(Location::local(&empty));
    wait_until(|| browser.active_location() == Some(Location::local(&empty)));
    wait_loaded(&browser, 0);
    focus_files(fixture);
}

#[test]
fn tenxer_yank_cut_and_unyank_mark_the_fill_or_cursor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_yank_cut_and_unyank_mark_the_fill_or_cursor",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::y);
            assert_eq!(clipboard_files(), [directory.join("a.txt")]);
            assert_eq!(mark(&directory, "a.txt"), ClipboardMark::Copy);
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::None);

            fill_b_and_c(&fixture);
            plain(&fixture, Key::y);
            assert_eq!(
                clipboard_files(),
                [directory.join("b.txt"), directory.join("c.txt")],
                "the fill wins over the cursor"
            );
            assert_eq!(mark(&directory, "a.txt"), ClipboardMark::None);
            assert_eq!(mark(&directory, "c.txt"), ClipboardMark::Copy);
            plain(&fixture, Key::x);
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::Cut);
            assert_eq!(mark(&directory, "c.txt"), ClipboardMark::Cut);

            shifted(&fixture, Key::Y);
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::None);
            assert!(clipboard_files().is_empty(), "unyank releases our payload");

            plain(&fixture, Key::y);
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::Copy);
            clipboard().set_text("external owner");
            wait_until(|| mark(&directory, "b.txt") == ClipboardMark::None);
            plain(&fixture, Key::x);
            shifted(&fixture, Key::X);
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::None);
            plain(&fixture, Key::y);
            clipboard().set_text("external owner");
            shifted(&fixture, Key::X);
            assert_eq!(
                clipboard_text().as_deref(),
                Some("external owner"),
                "another owner's clipboard is not wiped"
            );

            assert!(fixture.press(Key::c, ModifierType::CONTROL_MASK));
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::Copy);
            assert_eq!(
                directory_names(&directory),
                ["a.txt", "b.txt", "c.txt"],
                "copy and cut never move files before paste"
            );
            for name in ["a.txt", "b.txt", "c.txt"] {
                assert_eq!(contents(&directory.join(name)), "preview");
            }

            open_empty_folder(&fixture);
            plain(&fixture, Key::y);
            assert_eq!(feedback(&fixture), "Nothing to yank");
            plain(&fixture, Key::x);
            assert_eq!(feedback(&fixture), "Nothing to cut");
        },
    );
}

#[test]
fn tenxer_c_chord_copies_paths_and_names() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_c_chord_copies_paths_and_names",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::c);
            assert_eq!(fixture.shortcuts.chord().text(), "c-");
            wait_until(|| fixture.shortcuts.chord_options().is_some());
            assert_eq!(
                fixture.shortcuts.chord_options(),
                Some(vec![
                    ("c".to_owned(), "Copy path".to_owned()),
                    ("n".to_owned(), "Copy name".to_owned()),
                ])
            );
            plain(&fixture, Key::c);
            assert_eq!(fixture.shortcuts.armed_chord(), None);
            assert_eq!(
                clipboard_text(),
                Some(directory.join("a.txt").to_string_lossy().into_owned())
            );
            plain(&fixture, Key::c);
            plain(&fixture, Key::n);
            assert_eq!(clipboard_text().as_deref(), Some("a.txt"));

            fill_b_and_c(&fixture);
            plain(&fixture, Key::c);
            plain(&fixture, Key::n);
            assert_eq!(clipboard_text().as_deref(), Some("b.txt\nc.txt"));
            plain(&fixture, Key::c);
            plain(&fixture, Key::z);
            assert_eq!(feedback(&fixture), "Unknown chord");
            assert_eq!(
                fixture.shortcuts.open_prompt_kind(),
                None,
                "z stays in the chord"
            );
            shifted(&fixture, Key::Y);
            assert_eq!(
                clipboard_text().as_deref(),
                Some("b.txt\nc.txt"),
                "Y no longer copies a path"
            );

            open_empty_folder(&fixture);
            plain(&fixture, Key::c);
            plain(&fixture, Key::c);
            assert_eq!(feedback(&fixture), "Nothing to copy");
        },
    );
}

#[test]
fn tenxer_paste_focuses_the_preferred_conflict_choice() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_paste_focuses_the_preferred_conflict_choice",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let destination = directory.join("dest");
            std::fs::create_dir(&destination).expect("destination");
            std::fs::write(destination.join("a.txt"), "existing").expect("conflict");
            std::fs::write(destination.join("b.txt"), "existing").expect("cut conflict");
            fixture.view.refresh();
            wait_until(|| rendered_name(&fixture.view.widget(), "dest"));
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();

            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::y);
            enter_folder(&fixture, "dest");
            for (key, modifiers, label) in [
                (Key::p, ModifierType::empty(), "Keep Both"),
                (Key::P, ModifierType::SHIFT_MASK, "Replace"),
                (Key::v, ModifierType::CONTROL_MASK, "Replace"),
            ] {
                assert!(fixture.press(key, modifiers));
                wait_until(|| modal_visible(&fixture.overlay));
                wait_focused_button(&fixture, label);
                close_modal(&fixture);
                assert_eq!(contents(&destination.join("a.txt")), "existing");
                assert_eq!(directory_names(&destination), ["a.txt", "b.txt"]);
                focus_files(&fixture);
            }
            plain(&fixture, Key::p);
            wait_focused_button(&fixture, "Keep Both").emit_clicked();
            wait_until(|| directory_names(&destination).len() == 3);
            wait_until(|| !modal_visible(&fixture.overlay));
            assert_eq!(contents(&destination.join("a.txt")), "existing");
            assert_eq!(contents(&directory.join("a.txt")), "preview");

            focus_files(&fixture);
            leave_folder(&fixture);
            move_to_named(&fixture, &browser, "b.txt");
            plain(&fixture, Key::x);
            enter_folder(&fixture, "dest");
            plain(&fixture, Key::p);
            wait_until(|| modal_visible(&fixture.overlay));
            wait_focused_button(&fixture, "Replace");
            close_modal(&fixture);
            assert!(directory.join("b.txt").exists(), "cancel keeps the source");
            assert_eq!(contents(&destination.join("b.txt")), "existing");
            assert_eq!(mark(&directory, "b.txt"), ClipboardMark::Cut);

            focus_files(&fixture);
            leave_folder(&fixture);
            move_to_named(&fixture, &browser, "c.txt");
            plain(&fixture, Key::x);
            enter_folder(&fixture, "dest");
            plain(&fixture, Key::p);
            wait_until(|| destination.join("c.txt").exists() && !directory.join("c.txt").exists());
            wait_until(|| !modal_visible(&fixture.overlay));
            wait_until(|| mark(&directory, "c.txt") == ClipboardMark::None);
            assert!(clipboard_files().is_empty(), "a completed cut consumes it");

            focus_files(&fixture);
            wait_until(|| fixture.press(Key::z, ModifierType::CONTROL_MASK));
            wait_until(|| directory.join("c.txt").exists() && !destination.join("c.txt").exists());

            focus_files(&fixture);
            plain(&fixture, Key::p);
            assert_eq!(feedback(&fixture), "Nothing to paste");
        },
    );
}

#[test]
fn tenxer_delete_confirms_trash_and_permanent_deletion() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_delete_confirms_trash_and_permanent_deletion",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();

            for (key, modifiers) in [
                (Key::d, ModifierType::empty()),
                (Key::Delete, ModifierType::empty()),
            ] {
                move_to_named(&fixture, &browser, "a.txt");
                assert!(fixture.press(key, modifiers));
                wait_until(|| modal_visible(&fixture.overlay));
                wait_focused_button(&fixture, "Move to Trash");
                close_modal(&fixture);
                assert!(
                    directory.join("a.txt").exists(),
                    "{key:?} waits for confirmation"
                );
            }
            move_to_named(&fixture, &browser, "a.txt");
            plain(&fixture, Key::d);
            wait_focused_button(&fixture, "Move to Trash");
            assert!(modal_key(&fixture, Key::h));
            wait_focused_button(&fixture, "Cancel");
            assert!(modal_key(&fixture, Key::d), "d d confirms from any button");
            wait_until(|| !directory.join("a.txt").exists());
            wait_until(|| !modal_visible(&fixture.overlay));
            assert!(directory.join("b.txt").exists());
            focus_files(&fixture);
            wait_until(|| fixture.press(Key::z, ModifierType::CONTROL_MASK));
            wait_until(|| directory.join("a.txt").exists());

            for (key, modifiers) in [
                (Key::D, ModifierType::SHIFT_MASK),
                (Key::Delete, ModifierType::SHIFT_MASK),
            ] {
                move_to_named(&fixture, &browser, "b.txt");
                assert!(fixture.press(key, modifiers));
                wait_until(|| modal_visible(&fixture.overlay));
                let cancel = wait_focused_button(&fixture, "Cancel");
                pump(300);
                assert_eq!(
                    focused_button(&fixture).and_then(|button| button.label()),
                    Some("Cancel".into()),
                    "the size summary must not move focus to Permanently delete"
                );
                cancel.emit_clicked();
                wait_until(|| !modal_visible(&fixture.overlay));
                assert!(directory.join("b.txt").exists(), "{key:?} cancel keeps it");
            }
            move_to_named(&fixture, &browser, "b.txt");
            shifted(&fixture, Key::D);
            wait_focused_button(&fixture, "Cancel");
            assert!(!modal_key(&fixture, Key::d));
            pump(200);
            assert!(
                directory.join("b.txt").exists() && modal_visible(&fixture.overlay),
                "d never confirms permanent deletion"
            );
            close_modal(&fixture);
            focus_files(&fixture);
            move_to_named(&fixture, &browser, "b.txt");
            shifted(&fixture, Key::D);
            wait_until(|| {
                widget_with_class(fixture.overlay.upcast_ref(), "action-dialog-confirm")
                    .is_some_and(|confirm| confirm.is_sensitive())
            });
            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| !directory.join("b.txt").exists());
            assert_eq!(directory_names(&directory), ["a.txt", "c.txt"]);

            open_empty_folder(&fixture);
            plain(&fixture, Key::d);
            assert_eq!(feedback(&fixture), "Nothing to delete");
            assert!(!modal_visible(&fixture.overlay));
        },
    );
}

#[test]
fn tenxer_create_prompt_makes_exact_files_and_folders() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_create_prompt_makes_exact_files_and_folders",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let submit = |text: &str| {
                focus_files(&fixture);
                plain(&fixture, Key::a);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Create));
                assert_eq!(
                    fixture.shortcuts.prompt_label().as_deref(),
                    Some("create \u{203a}")
                );
                fixture.shortcuts.prompt().set_text(text);
                plain(&fixture, Key::Return);
            };

            for name in ["new.txt", "  spaced  ", "日本語 ✓"] {
                submit(name);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                wait_until(|| directory.join(name).is_file());
                assert_eq!(contents(&directory.join(name)), "");
            }
            submit("folder/");
            wait_until(|| directory.join("folder").is_dir());

            std::fs::write(directory.join("new.txt"), "keep").expect("existing contents");
            std::os::unix::fs::symlink(directory.join("missing"), directory.join("link"))
                .expect("dangling link");
            for name in ["new.txt", "folder/", "folder", "link", "link/"] {
                submit(name);
                let bare = name.trim_end_matches('/');
                assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Create));
                assert_eq!(
                    fixture.shortcuts.prompt_hint(),
                    Some(format!("\u{201c}{bare}\u{201d} already exists"))
                );
                assert_eq!(
                    fixture.shortcuts.prompt_text(),
                    name,
                    "the name can be fixed"
                );
                plain(&fixture, Key::Escape);
            }
            assert_eq!(contents(&directory.join("new.txt")), "keep");
            assert!(
                !directory.join("missing").exists(),
                "links are not followed"
            );

            let names = directory_names(&directory);
            for (text, hint) in [
                ("", "Enter a name"),
                ("   ", "Enter a name"),
                ("/", "Enter a name"),
                (".", "That name is reserved"),
                ("../", "That name is reserved"),
                ("nested/name", "Names cannot contain /"),
                ("/absolute", "Names cannot contain /"),
                ("twice//", "Names cannot contain /"),
            ] {
                submit(text);
                assert_eq!(
                    fixture.shortcuts.prompt_hint().as_deref(),
                    Some(hint),
                    "{text:?}"
                );
                fixture.shortcuts.prompt().set_text("edited");
                assert_eq!(
                    fixture.shortcuts.prompt_hint(),
                    None,
                    "editing clears the hint"
                );
                plain(&fixture, Key::Escape);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            }
            assert_eq!(
                fixture.view.create_typed_entry("nul\0name"),
                Err(CreateRefusal::Invalid(
                    "Names cannot contain NUL characters"
                ))
            );

            let pending = |fixture: &KeyboardFixture, name: &str| {
                focus_files(fixture);
                plain(fixture, Key::a);
                fixture.shortcuts.prompt().set_text(name);
            };
            pending(&fixture, "escaped.txt");
            plain(&fixture, Key::Escape);
            assert!(fixture.view.item_view_has_focus());
            pending(&fixture, "clicked.txt");
            assert!(press_file_row(&fixture.view.widget(), "b.txt"));
            focus_files(&fixture);
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            assert!(fixture.shortcuts.prompt().text().is_empty());
            pending(&fixture, "replaced.txt");
            fixture.shortcuts.open_prompt(Prompt::Find);
            assert!(fixture.shortcuts.prompt().text().is_empty());
            plain(&fixture, Key::Escape);
            pending(&fixture, "mode-exit.txt");
            PreferenceManager::shared().set_tenxer_mode(false);
            pump(50);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            pump(200);
            assert_eq!(
                directory_names(&directory),
                names,
                "discarded prompts create nothing"
            );

            focus_files(&fixture);
            assert!(fixture.press(
                Key::n,
                ModifierType::CONTROL_MASK | ModifierType::SHIFT_MASK
            ));
            assert!(
                fixture.view.new_entry_is_active(),
                "Ctrl+Shift+N keeps inline naming"
            );
            plain(&fixture, Key::Escape);
        },
    );
}

#[test]
fn ctrl_alt_n_groups_selected_items_into_a_named_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::ctrl_alt_n_groups_selected_items_into_a_named_folder",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            assert!(
                browser.select_entries_by_name_at(0, &["a.txt".to_owned(), "b.txt".to_owned()],)
            );
            focus_files(&fixture);
            assert!(fixture.press(Key::n, ModifierType::CONTROL_MASK | ModifierType::ALT_MASK));
            wait_until(|| directory.join("new folder").is_dir());
            wait_until(|| {
                directory.join("new folder/a.txt").is_file()
                    && directory.join("new folder/b.txt").is_file()
            });
            wait_until(|| fixture.view.rename_is_active());
            assert!(!directory.join("a.txt").exists());
            assert!(!directory.join("b.txt").exists());
            assert_eq!(
                directory_names(&directory.join("new folder")),
                ["a.txt", "b.txt"]
            );

            let field = fixture.view.active_rename_field().expect("rename field");
            field.set_text("grouped");
            field.emit_activate();
            wait_until(|| directory.join("grouped/b.txt").is_file());
            assert!(!directory.join("new folder").exists());

            wait_until(|| fixture.press(Key::z, ModifierType::CONTROL_MASK));
            wait_until(|| {
                directory.join("a.txt").is_file()
                    && directory.join("b.txt").is_file()
                    && !directory.join("grouped").exists()
            });
            assert!(
                !directory.join("new folder").exists(),
                "one undo reverts the whole gesture"
            );

            open_empty_folder(&fixture);
            let empty = directory.join("empty");
            assert!(fixture.press(Key::n, ModifierType::CONTROL_MASK | ModifierType::ALT_MASK));
            wait_until(|| empty.join("new folder").is_dir());
            wait_until(|| fixture.view.rename_is_active());
            assert_eq!(directory_names(&empty), ["new folder"]);
            plain(&fixture, Key::Escape);
        },
    );
}

#[test]
fn abandoned_group_naming_keeps_a_later_rename_separate() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::abandoned_group_naming_keeps_a_later_rename_separate",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                for dismissal in ["pending-escape", "unmap", "rename-failure"] {
                    let fixture = KeyboardFixture::new();
                    fixture.view.set_view_mode(mode);
                    let directory = fixture._directory.path();
                    let browser = fixture.view.browser();
                    wait_until(|| {
                        browser
                            .select_entries_by_name_at(0, &["a.txt".to_owned(), "b.txt".to_owned()])
                    });
                    if dismissal == "pending-escape" {
                        let keys = RefCell::new(Some(fixture.keys.clone()));
                        browser.observe(move |event| {
                            if matches!(event, BrowserEvent::TransferStarted { .. })
                                && let Some(keys) = keys.take()
                            {
                                assert!(keys.emit_by_name::<bool>(
                                    "key-pressed",
                                    &[&Key::Escape, &0u32, &ModifierType::empty()]
                                ));
                            }
                        });
                    }
                    focus_files(&fixture);
                    assert!(
                        fixture.press(Key::n, ModifierType::CONTROL_MASK | ModifierType::ALT_MASK)
                    );
                    wait_until(|| {
                        directory.join("new folder/a.txt").exists()
                            && directory.join("new folder/b.txt").exists()
                    });
                    if dismissal != "pending-escape" {
                        wait_until(|| fixture.view.rename_is_active());
                        let field = fixture.view.active_rename_field().expect("gesture field");
                        if dismissal == "unmap" {
                            field.set_visible(false);
                        } else {
                            field.set_text("c.txt");
                            field.emit_activate();
                            wait_until(|| modal_visible(&fixture.overlay));
                            close_modal(&fixture);
                        }
                        wait_until(|| !fixture.view.rename_is_active());
                    }
                    wait_until(|| browser.select_entries_by_name_at(0, &["new folder".to_owned()]));
                    focus_files(&fixture);
                    plain(&fixture, Key::F2);
                    wait_until(|| fixture.view.rename_is_active());
                    let field = fixture.view.active_rename_field().expect("later rename");
                    field.set_text("later");
                    field.emit_activate();
                    wait_until(|| directory.join("later/a.txt").exists());
                    wait_until(|| fixture.press(Key::z, ModifierType::CONTROL_MASK));
                    wait_until(|| directory.join("new folder/a.txt").exists());
                    assert!(directory.join("new folder/b.txt").exists());
                    assert!(!directory.join("a.txt").exists());
                    assert!(!directory.join("later").exists());
                    wait_until(|| fixture.press(Key::z, ModifierType::CONTROL_MASK));
                    wait_until(|| {
                        directory.join("a.txt").exists()
                            && directory.join("b.txt").exists()
                            && !directory.join("new folder").exists()
                    });
                }
            }
        },
    );
}

#[test]
fn tenxer_move_and_copy_prompts_send_targets_to_a_picked_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_move_and_copy_prompts_send_targets_to_a_picked_folder",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let destination = directory.join("dest");
            let inner = destination.join("inner");
            let other = directory.join("other");
            std::fs::create_dir_all(&inner).expect("destination");
            std::fs::create_dir(&other).expect("sibling destination");
            fixture.view.refresh();
            wait_until(|| rendered_name(&fixture.view.widget(), "dest"));
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let open = |key: Key, kind: Prompt, label: &str| {
                focus_files(&fixture);
                shifted(&fixture, key);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(kind));
                assert_eq!(fixture.shortcuts.prompt_label().as_deref(), Some(label));
            };
            let listed = |text: &str, expected: &[&Path]| {
                fixture.shortcuts.prompt().set_text(text);
                wait_until(|| fixture.shortcuts.candidates() == expected);
                assert!(fixture.shortcuts.candidates_shown(), "{text:?}");
            };
            let hint = || fixture.shortcuts.prompt_hint();
            let refused = |text: &str, expected: &str| {
                fixture.shortcuts.prompt().set_text(text);
                plain(&fixture, Key::Return);
                wait_until(|| hint().as_deref() == Some(expected));
                assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::MoveTo));
            };

            fill_b_and_c(&fixture);
            open(Key::C, Prompt::CopyTo, "copy to \u{203a}");
            listed("dest", &[&destination, &inner]);
            assert_eq!(hint().as_deref(), Some("1 of 2"));
            plain(&fixture, Key::Return);
            wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
            wait_until(|| directory_names(&destination) == ["b.txt", "c.txt", "inner"]);
            assert!(directory.join("b.txt").exists() && directory.join("c.txt").exists());
            assert_eq!(browser.active_location(), origin, "copying stays put");

            browser.clear_active_selection();
            move_to_named(&fixture, &browser, "a.txt");
            open(Key::M, Prompt::MoveTo, "move to \u{203a}");
            listed("dest in", &[&inner]);
            plain(&fixture, Key::Return);
            wait_until(|| inner.join("a.txt").exists() && !directory.join("a.txt").exists());
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert_eq!(browser.active_location(), origin, "moving stays put");
            wait_until(|| focused_name(&browser) == "b.txt");
            assert!(
                fill_names(&browser).is_empty(),
                "the neighbor is cursor-only"
            );
            plain(&fixture, Key::j);
            assert_eq!(focused_name(&browser), "c.txt");
            assert!(fill_names(&browser).is_empty());

            browser.clear_active_selection();
            move_to_named(&fixture, &browser, "b.txt");
            open(Key::M, Prompt::MoveTo, "move to \u{203a}");
            plain(&fixture, Key::Return);
            assert_eq!(
                fixture.shortcuts.open_prompt_kind(),
                None,
                "empty Enter closes"
            );
            for (text, expected) in [
                ("nowhere", "No matching folders"),
                ("smb://host/share", "Only local folders can be chosen"),
                ("~someone/x", "Only ~ and ~/ are supported"),
                ("./", "Already in this folder"),
                ("./c.txt/", "Not a folder"),
                ("./nowhere/", "No such folder"),
            ] {
                open(Key::M, Prompt::MoveTo, "move to \u{203a}");
                plain(&fixture, Key::Down);
                assert_eq!(focused_name(&browser), "b.txt", "the target is fixed");
                refused(text, expected);
                plain(&fixture, Key::Escape);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            }
            assert!(directory.join("b.txt").exists(), "refusals move nothing");

            open(Key::M, Prompt::MoveTo, "move to \u{203a}");
            fixture.shortcuts.prompt().set_text("./");
            wait_until(|| {
                let mut found = fixture.shortcuts.candidates();
                found.sort();
                found == [destination.clone(), inner.clone(), other.clone()]
            });
            plain(&fixture, Key::Escape);

            move_to_named(&fixture, &browser, "dest");
            open(Key::M, Prompt::MoveTo, "move to \u{203a}");
            fixture.shortcuts.prompt().set_text("inner");
            wait_until(|| hint().as_deref() == Some("No matching folders"));
            assert!(
                fixture.shortcuts.candidates().is_empty(),
                "a folder is never offered inside itself"
            );
            refused("./dest/inner/", "Can\u{2019}t put a folder inside itself");
            plain(&fixture, Key::Escape);
            open(Key::C, Prompt::CopyTo, "copy to \u{203a}");
            listed("./dest/../other/", &[&other]);
            plain(&fixture, Key::Return);
            wait_until(|| other.join("dest/inner/a.txt").exists());
            assert!(inner.join("a.txt").exists(), "copy preserves the source");

            move_to_named(&fixture, &browser, "b.txt");
            open(Key::C, Prompt::CopyTo, "copy to \u{203a}");
            let typed = format!("{}/", destination.display());
            listed(&typed, &[&destination, &inner]);
            plain(&fixture, Key::Return);
            wait_until(|| modal_visible(&fixture.overlay));
            wait_focused_button(&fixture, "Keep Both");
            close_modal(&fixture);
            assert_eq!(directory_names(&destination), ["b.txt", "c.txt", "inner"]);

            move_to_named(&fixture, &browser, "c.txt");
            open(Key::M, Prompt::MoveTo, "move to \u{203a}");
            fixture.shortcuts.prompt().set_text("inner");
            wait_until(|| fixture.shortcuts.candidates().len() == 2);
            let mut found = fixture.shortcuts.candidates();
            found.sort();
            assert_eq!(found, [inner.clone(), other.join("dest/inner")]);
            for (key, position) in [
                (Key::Up, "2 of 2"),
                (Key::Down, "1 of 2"),
                (Key::Down, "2 of 2"),
            ] {
                plain(&fixture, key);
                assert_eq!(hint().as_deref(), Some(position), "{key:?}");
            }
            assert_eq!(focused_name(&browser), "c.txt", "stepping keeps the cursor");
            let chosen = fixture.shortcuts.candidates()[1].clone();
            let later = fixture.shortcuts.candidates();
            fixture
                .shortcuts
                .show_candidates(later, CandidateKeys::default());
            assert_eq!(
                fixture.shortcuts.chosen_candidate().as_ref(),
                Some(&chosen),
                "results that arrive later keep the stepped choice"
            );
            let relative = chosen.strip_prefix(&directory).expect("candidate below");
            plain(&fixture, Key::Tab);
            assert_eq!(
                fixture.shortcuts.prompt_text(),
                format!("./{}/", relative.display()),
                "Tab writes the chosen folder into the prompt"
            );
            assert!(directory.join("c.txt").exists(), "Tab never sends");
            wait_until(|| fixture.shortcuts.candidates() == [chosen.clone()]);
            plain(&fixture, Key::Return);
            wait_until(|| chosen.join("c.txt").exists() && !directory.join("c.txt").exists());

            move_to_named(&fixture, &browser, "b.txt");
            open(Key::M, Prompt::MoveTo, "move to \u{203a}");
            listed("^other$", &[&other]);
            plain(&fixture, Key::Return);
            plain(&fixture, Key::Escape);
            pump(200);
            assert!(
                directory.join("b.txt").exists(),
                "cancelled validation must not move"
            );
            assert!(!other.join("b.txt").exists());

            open(Key::C, Prompt::CopyTo, "copy to \u{203a}");
            listed("^other$", &[&other]);
            fixture.shortcuts.click_candidate(0);
            wait_until(|| other.join("b.txt").exists());
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert!(directory.join("b.txt").exists());

            open_empty_folder(&fixture);
            for (key, message) in [(Key::M, "Nothing to move"), (Key::C, "Nothing to copy")] {
                shifted(&fixture, key);
                assert_eq!(feedback(&fixture), message);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            }
        },
    );
}

/// Headless GVfs cannot list Trash; restore behavior is covered by the trash tests.
#[test]
fn tenxer_restore_key_refuses_items_outside_trash() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_restore_key_refuses_items_outside_trash",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();

            fill_b_and_c(&fixture);
            shifted(&fixture, Key::R);
            assert_eq!(feedback(&fixture), "Only items in Trash can be restored");
            assert!(!modal_visible(&fixture.overlay));

            let trash = Location::uri("trash:///");
            browser.navigate(trash.clone());
            wait_until(|| browser.active_location() == Some(trash.clone()));
            wait_loaded(&browser, 0);
            focus_files(&fixture);
            shifted(&fixture, Key::R);
            assert_eq!(feedback(&fixture), "Nothing to restore");
            assert!(!modal_visible(&fixture.overlay));
        },
    );
}

#[test]
fn tenxer_action_chord_compresses_and_extracts_archives() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::tenxer_action_chord_compresses_and_extracts_archives",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let source = tempfile::tempdir().expect("archive source");
            std::fs::write(source.path().join("notes.txt"), "notes").expect("archived file");
            crate::adapters::write_compression_fixture(
                &directory.join("bundle.tar"),
                &[source.path().join("notes.txt")],
                crate::services::ArchiveFormat::Tar,
                None,
            )
            .expect("fixture archive");
            let destination = directory.join("dest");
            std::fs::create_dir(&destination).expect("destination");
            std::fs::create_dir(directory.join("nested.zip")).expect("folder named like an archive");
            fixture.view.refresh();
            wait_until(|| {
                rendered_name(&fixture.view.widget(), "bundle.tar")
                    && rendered_name(&fixture.view.widget(), "nested.zip")
            });
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let origin = browser.active_location();
            let action = |key: Key, modifiers: ModifierType| {
                focus_files(&fixture);
                plain(&fixture, Key::semicolon);
                assert!(fixture.press(key, modifiers), "; {key:?}");
                assert_eq!(fixture.shortcuts.armed_chord(), None);
            };

            move_to_named(&fixture, &browser, "a.txt");
            action(Key::e, ModifierType::empty());
            assert_eq!(feedback(&fixture), "Not an archive");
            action(Key::c, ModifierType::empty());
            wait_until(|| modal_visible(&fixture.overlay));
            close_modal(&fixture);
            assert_eq!(directory_names(&destination), Vec::<String>::new());

            move_to_named(&fixture, &browser, "nested.zip");
            action(Key::e, ModifierType::empty());
            assert_eq!(feedback(&fixture), "Not an archive");
            action(Key::E, ModifierType::SHIFT_MASK);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
            assert_eq!(feedback(&fixture), "Not an archive");

            move_to_named(&fixture, &browser, "bundle.tar");
            action(Key::e, ModifierType::empty());
            wait_until(|| std::fs::read_to_string(directory.join("notes.txt")).is_ok());
            assert_eq!(contents(&directory.join("notes.txt")), "notes");
            assert_eq!(browser.active_location(), origin);
            wait_until(|| fill_names(&browser) == ["notes.txt"]);

            browser.clear_active_selection();
            move_to_named(&fixture, &browser, "bundle.tar");
            action(Key::E, ModifierType::SHIFT_MASK);
            assert_eq!(
                fixture.shortcuts.open_prompt_kind(),
                Some(Prompt::ExtractTo)
            );
            assert_eq!(
                fixture.shortcuts.prompt_label().as_deref(),
                Some("extract to \u{203a}")
            );
            fixture.shortcuts.prompt().set_text("missing");
            wait_until(|| {
                fixture.shortcuts.prompt_hint().as_deref() == Some("No matching folders")
            });
            plain(&fixture, Key::Return);
            assert_eq!(
                fixture.shortcuts.prompt_hint().as_deref(),
                Some("No matching folders")
            );
            fixture.shortcuts.prompt().set_text("dest");
            wait_until(|| fixture.shortcuts.candidates() == [destination.clone()]);
            plain(&fixture, Key::Return);
            wait_until(|| std::fs::read_to_string(destination.join("notes.txt")).is_ok());
            assert_eq!(contents(&destination.join("notes.txt")), "notes");
            assert_eq!(browser.active_location(), origin, "extracting stays put");
            pump(500);
            assert!(
                fill_names(&browser).is_empty(),
                "the open folder's notes.txt is not the extracted item"
            );

            for name in ["b.txt", "bundle.tar"] {
                move_to_named(&fixture, &browser, name);
                plain(&fixture, Key::space);
            }
            assert_eq!(fill_names(&browser), ["b.txt", "bundle.tar"]);
            action(Key::e, ModifierType::empty());
            assert_eq!(feedback(&fixture), "Extract one archive at a time");
            browser.clear_active_selection();

            open_empty_folder(&fixture);
            for (key, message) in [
                (Key::c, "Nothing to compress"),
                (Key::e, "Nothing to extract"),
            ] {
                action(key, ModifierType::empty());
                assert_eq!(feedback(&fixture), message, "; {key:?}");
            }
        },
    );
}
