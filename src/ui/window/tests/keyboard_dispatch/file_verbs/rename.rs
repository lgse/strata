// SPDX-License-Identifier: MIT

use super::*;

fn open_rename(fixture: &KeyboardFixture, key: Key) {
    focus_files(fixture);
    plain(fixture, key);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Rename));
    assert_eq!(
        fixture.shortcuts.prompt_label().as_deref(),
        Some("rename \u{203a}")
    );
}

fn submit_rename(fixture: &KeyboardFixture, text: &str) {
    open_rename(fixture, Key::r);
    fixture.shortcuts.prompt().set_text(text);
    plain(fixture, Key::Return);
}

/// `r` and F2 fill the prompt with the cursor's name, stem selected, and the
/// target stays fixed while the prompt is open.
fn prompt_targets_the_cursor(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    for key in [Key::r, Key::F2] {
        open_rename(fixture, key);
        assert_eq!(fixture.shortcuts.prompt_text(), "a.txt", "{key:?}");
        assert_eq!(
            fixture.shortcuts.prompt().selection_bounds(),
            Some((0, 1)),
            "the stem is selected"
        );
        plain(fixture, Key::Down);
        assert_eq!(focused_name(&browser), "a.txt", "the target stays put");
        plain(fixture, Key::Escape);
        assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
        assert!(fixture.view.item_view_has_focus());
    }
}

fn renames_only_the_cursor(fixture: &KeyboardFixture, directory: &Path) {
    submit_rename(fixture, "a.txt");
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    submit_rename(fixture, "renamed.txt");
    wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
    wait_until(|| directory.join("renamed.txt").is_file());
    assert!(!directory.join("a.txt").exists());
    assert_eq!(contents(&directory.join("renamed.txt")), "preview");
    assert!(directory.join("c.txt").is_file(), "the fill is not renamed");
    wait_until(|| focused_name(&fixture.view.browser()) == "renamed.txt");
}

fn refusals_keep_the_name_editable(fixture: &KeyboardFixture, directory: &Path) {
    let too_long = "x".repeat(300);
    for (text, hint) in [
        ("", "Enter a name"),
        ("  ", "Enter a name"),
        ("..", "That name is reserved"),
        ("b.txt/name", "\u{201c}b.txt\u{201d} is not a folder"),
        (
            "renamed.txt/",
            "An existing file can\u{2019}t become a folder",
        ),
        ("nested//name", "Enter a name"),
        ("b.txt", "\u{201c}b.txt\u{201d} already exists"),
        (&too_long, "The name is too long"),
    ] {
        submit_rename(fixture, text);
        wait_until(|| fixture.shortcuts.prompt_hint().as_deref() == Some(hint));
        assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Rename));
        assert_eq!(
            fixture.shortcuts.prompt_text(),
            text,
            "the name can be fixed"
        );
        plain(fixture, Key::Escape);
    }
    assert_eq!(contents(&directory.join("b.txt")), "preview");
    assert!(directory.join("renamed.txt").is_file());
}

fn renames_a_folder_with_its_whole_name_selected(fixture: &KeyboardFixture, directory: &Path) {
    std::fs::create_dir(directory.join("folder")).expect("folder");
    std::fs::write(directory.join("folder/inside.txt"), "kept").expect("child");
    refresh(fixture, "folder");
    move_to_named(fixture, &fixture.view.browser(), "folder");
    open_rename(fixture, Key::F2);
    assert_eq!(
        fixture.shortcuts.prompt().selection_bounds(),
        Some((0, 6)),
        "a folder's whole name is selected"
    );
    fixture.shortcuts.prompt().set_text("folder.d");
    plain(fixture, Key::Return);
    wait_until(|| directory.join("folder.d/inside.txt").is_file());
    assert_eq!(contents(&directory.join("folder.d/inside.txt")), "kept");
}

fn a_read_only_folder_reports_the_failure(fixture: &KeyboardFixture, directory: &Path) {
    move_to_named(fixture, &fixture.view.browser(), "b.txt");
    let writable = std::fs::metadata(directory).expect("fixture").permissions();
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o555))
        .expect("read-only fixture");
    // Root ignores directory permissions.
    if std::fs::write(directory.join("probe"), "").is_err() {
        focus_files(fixture);
        submit_rename(fixture, "denied.txt");
        wait_until(|| modal_visible(&fixture.overlay));
        assert!(click_class(&fixture.overlay, "action-dialog-close"));
        wait_until(|| !modal_visible(&fixture.overlay));
        assert!(directory.join("b.txt").is_file());
        assert!(!directory.join("denied.txt").exists());
    }
    std::fs::set_permissions(directory, writable).expect("restore fixture");
}

fn a_nested_name_moves_into_a_new_folder(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, "b.txt");
    submit_rename(fixture, "nested/b.txt");
    wait_until(|| directory.join("nested/b.txt").is_file());
    assert!(!directory.join("b.txt").exists());
    assert_eq!(contents(&directory.join("nested/b.txt")), "preview");
    wait_until(|| focused_name(&browser) == "nested");
}

#[test]
fn tenxer_footer_rename_changes_only_the_focused_item_on_enter() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::rename::tenxer_footer_rename_changes_only_the_focused_item_on_enter",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            move_to_named(&fixture, &browser, "c.txt");
            plain(&fixture, Key::space);
            move_to_named(&fixture, &browser, "a.txt");
            assert_eq!(fill_names(&browser), ["c.txt"]);

            prompt_targets_the_cursor(&fixture);
            assert!(directory.join("a.txt").is_file());
            renames_only_the_cursor(&fixture, &directory);
            refusals_keep_the_name_editable(&fixture, &directory);

            let exact = " spaced 日本語 ✓ ";
            submit_rename(&fixture, exact);
            wait_until(|| directory.join(exact).is_file());
            assert_eq!(contents(&directory.join(exact)), "preview");

            renames_a_folder_with_its_whole_name_selected(&fixture, &directory);
            a_read_only_folder_reports_the_failure(&fixture, &directory);
            a_nested_name_moves_into_a_new_folder(&fixture, &directory);
        },
    );
}

#[test]
fn typed_rename_drops_a_dismissed_request_and_refuses_a_hard_link_name() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::rename::typed_rename_drops_a_dismissed_request_and_refuses_a_hard_link_name",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();
            std::fs::hard_link(directory.join("a.txt"), directory.join("link.txt"))
                .expect("hard link");
            refresh(&fixture, "link.txt");
            move_to_named(&fixture, &browser, "a.txt");

            submit_rename(&fixture, "later.txt");
            plain(&fixture, Key::Escape);
            open_rename(&fixture, Key::r);
            fixture.shortcuts.prompt().set_text("later.txt");
            pump(200);
            assert!(
                !directory.join("later.txt").exists(),
                "a dismissed request must not submit the reopened prompt"
            );
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Rename));
            plain(&fixture, Key::Escape);

            submit_rename(&fixture, "link.txt");
            wait_until(|| {
                fixture.shortcuts.prompt_hint().as_deref()
                    == Some("\u{201c}link.txt\u{201d} already exists")
            });
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Rename));
            assert!(!modal_visible(&fixture.overlay));
            plain(&fixture, Key::Escape);
            assert_eq!(
                [&directory.join("a.txt"), &directory.join("link.txt")].map(|path| contents(path)),
                ["preview", "preview"]
            );
        },
    );
}

fn type_pending_rename(fixture: &KeyboardFixture) {
    move_to_named(fixture, &fixture.view.browser(), "a.txt");
    open_rename(fixture, Key::r);
    fixture.shortcuts.prompt().set_text("stale.txt");
}

/// A row click, another prompt, and leaving tenxer mode each discard a typed rename.
fn discarded_renames_rename_nothing(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    let names = directory_names(directory);
    type_pending_rename(fixture);
    assert!(press_file_row(&fixture.view.widget(), "b.txt"));
    focus_files(fixture);
    wait_until(|| fixture.shortcuts.open_prompt_kind().is_none());
    assert_eq!(focused_name(&browser), "b.txt", "the click keeps its row");

    type_pending_rename(fixture);
    fixture.shortcuts.open_prompt(Prompt::Find);
    assert!(fixture.shortcuts.prompt_text().is_empty());
    fixture.shortcuts.prompt().set_text("c");
    plain(fixture, Key::Return);
    assert_eq!(focused_name(&browser), "c.txt");

    type_pending_rename(fixture);
    PreferenceManager::shared().set_tenxer_mode(false);
    pump(50);
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    pump(200);
    assert_eq!(directory_names(directory), names, "nothing was renamed");
}

fn default_map_keeps_inline_rename(fixture: &KeyboardFixture) {
    move_to_named(fixture, &fixture.view.browser(), "a.txt");
    assert!(fixture.press(Key::F2, ModifierType::empty()));
    assert!(
        fixture.view.rename_is_active(),
        "the default map keeps inline rename"
    );
    assert!(fixture.press(Key::Escape, ModifierType::empty()));
    assert!(!fixture.view.rename_is_active());
}

#[test]
fn tenxer_footer_rename_leaves_no_stale_target() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_verbs::rename::tenxer_footer_rename_leaves_no_stale_target",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();

            discarded_renames_rename_nothing(&fixture, &directory);
            default_map_keeps_inline_rename(&fixture);

            PreferenceManager::shared().set_tenxer_mode(true);
            pump(50);
            super::super::footer_prompt::commit_filter(&fixture, "b");
            wait_until(|| fixture.view.filter_result_names() == ["b.txt"]);
            open_rename(&fixture, Key::r);
            assert_eq!(
                fixture.shortcuts.prompt_text(),
                "b.txt",
                "the focused result"
            );
            plain(&fixture, Key::Escape);
            plain(&fixture, Key::Escape);

            open_empty_folder(&fixture);
            for key in [Key::r, Key::F2] {
                plain(&fixture, key);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                assert_eq!(feedback(&fixture), "Nothing to rename");
            }
        },
    );
}
