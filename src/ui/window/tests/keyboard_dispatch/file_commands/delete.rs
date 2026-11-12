// SPDX-License-Identifier: MIT

use super::*;

/// **d** and Delete both ask before moving to Trash.
fn trash_keys_wait_for_confirmation(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    for (key, modifiers) in [
        (Key::d, ModifierType::empty()),
        (Key::Delete, ModifierType::empty()),
    ] {
        move_to_named(fixture, &browser, "a.txt");
        assert!(fixture.press(key, modifiers));
        wait_until(|| modal_visible(&fixture.overlay));
        wait_focused_button(fixture, "Move to Trash");
        close_modal(fixture);
        assert!(
            directory.join("a.txt").exists(),
            "{key:?} waits for confirmation"
        );
    }
}

/// `d d` confirms from any button; Ctrl+Z brings the item back.
fn d_d_confirms_and_undo_restores(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, "a.txt");
    plain(fixture, Key::d);
    wait_focused_button(fixture, "Move to Trash");
    assert!(modal_key(fixture, Key::h));
    wait_focused_button(fixture, "Cancel");
    assert!(modal_key(fixture, Key::d), "d d confirms from any button");
    wait_until(|| !directory.join("a.txt").exists());
    wait_until(|| !modal_visible(&fixture.overlay));
    assert!(directory.join("b.txt").exists());
    focus_files(fixture);
    undo(fixture);
    wait_until(|| directory.join("a.txt").exists());
}

/// Shift+D and Shift+Delete focus Permanently delete, which the size summary
/// must not steal; Escape cancels.
fn permanent_keys_cancel_with_escape(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    for (key, modifiers) in [
        (Key::D, ModifierType::SHIFT_MASK),
        (Key::Delete, ModifierType::SHIFT_MASK),
    ] {
        move_to_named(fixture, &browser, "b.txt");
        assert!(fixture.press(key, modifiers));
        wait_until(|| modal_visible(&fixture.overlay));
        wait_confirm_focused(fixture);
        pump(300);
        assert!(
            confirm_button(fixture).is_some_and(|confirm| confirm.has_focus()),
            "the size summary must keep focus on Permanently delete"
        );
        assert!(modal_key(fixture, Key::Escape));
        wait_until(|| !modal_visible(&fixture.overlay));
        assert!(directory.join("b.txt").exists(), "{key:?} cancel keeps it");
    }
}

fn d_never_confirms_permanent_deletion(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, "b.txt");
    shifted(fixture, Key::D);
    wait_confirm_focused(fixture);
    assert!(!modal_key(fixture, Key::d));
    pump(200);
    assert!(
        directory.join("b.txt").exists() && modal_visible(&fixture.overlay),
        "d never confirms permanent deletion"
    );
    close_modal(fixture);
    focus_files(fixture);
}

fn enter_confirms_permanent_deletion(fixture: &KeyboardFixture, directory: &Path) {
    let browser = fixture.view.browser();
    move_to_named(fixture, &browser, "b.txt");
    shifted(fixture, Key::D);
    wait_confirm_focused(fixture);
    assert!(modal_key(fixture, Key::Return));
    wait_until(|| !directory.join("b.txt").exists());
    assert_eq!(directory_names(directory), ["a.txt", "c.txt"]);
}

#[test]
fn tenxer_delete_confirms_trash_and_permanent_deletion() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::delete::tenxer_delete_confirms_trash_and_permanent_deletion",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();

            trash_keys_wait_for_confirmation(&fixture, &directory);
            d_d_confirms_and_undo_restores(&fixture, &directory);
            permanent_keys_cancel_with_escape(&fixture, &directory);
            d_never_confirms_permanent_deletion(&fixture, &directory);
            enter_confirms_permanent_deletion(&fixture, &directory);

            open_empty_folder(&fixture);
            plain(&fixture, Key::d);
            assert_eq!(feedback(&fixture), "Nothing to delete");
            assert!(!modal_visible(&fixture.overlay));
        },
    );
}

/// Shift+Delete asks, and the confirmation deletes `name` permanently.
fn shift_delete_removes(fixture: &KeyboardFixture, directory: &Path, name: &str) {
    assert!(fixture.press(Key::Delete, ModifierType::SHIFT_MASK));
    wait_until(|| modal_visible(&fixture.overlay));
    wait_confirm_sensitive(fixture);
    assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
    wait_until(|| !directory.join(name).exists());
}

#[test]
fn columns_shift_delete_deletes_the_cursor_when_the_fill_is_empty() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::delete::columns_shift_delete_deletes_the_cursor_when_the_fill_is_empty",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            let browser = fixture.view.browser();

            browser.select_first_on_load(0);
            browser.reload_active();
            wait_until(|| browser.selection_is_load_cursor());
            browser.place_cursor(0, 2, None);
            assert_eq!(focused_name(&browser), "c.txt");
            focus_files(&fixture);

            shift_delete_removes(&fixture, &directory, "c.txt");
            assert_eq!(directory_names(&directory), ["a.txt", "b.txt"]);
        },
    );
}

#[test]
fn columns_shift_delete_targets_the_open_folder_when_the_child_selects_nothing() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::delete::columns_shift_delete_targets_the_open_folder_when_the_child_selects_nothing",
        || {
            let fixture = KeyboardFixture::new();
            let directory = fixture._directory.path().to_path_buf();
            seed_folder(&fixture, "folder");
            activate_folder(&fixture, "folder");

            shift_delete_removes(&fixture, &directory, "folder");
            assert_eq!(directory_names(&directory), ["a.txt", "b.txt", "c.txt"]);
        },
    );
}

#[test]
fn closing_a_cancelled_folder_deletion_focuses_the_folder_row() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::delete::closing_a_cancelled_folder_deletion_focuses_the_folder_row",
        || {
            let held = Rc::new(HeldOperations::default());
            let fixture = held_fixture(&held, Rc::new(crate::adapters::LocalFileSource));
            // As in a real window: the reload can drop the row the dialog opened from.
            fixture
                .view
                .set_as_modal_focus_fallback(fixture.window.upcast_ref());
            let folder = seed_folder(&fixture, "folder");
            std::fs::write(folder.join("inner.txt"), b"kept").expect("folder contents");
            let browser = fixture.view.browser();
            activate_folder(&fixture, "folder");

            assert!(fixture.press(Key::Delete, ModifierType::SHIFT_MASK));
            wait_until(|| modal_visible(&fixture.overlay));
            wait_confirm_sensitive(&fixture);
            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| !held.delete_requests.borrow().is_empty());
            let (id, ..) = held.delete_requests.borrow_mut().remove(0);
            let parent = Location::local(fixture._directory.path());
            held.emit(
                id,
                crate::services::OperationEvent::Cancelled {
                    request_id: id,
                    result: crate::services::CancelledOperation {
                        not_attempted: vec![Location::local(&folder)],
                        affected_locations: std::collections::HashSet::from([parent]),
                        ..Default::default()
                    },
                },
            );
            wait_until(|| modal_visible(&fixture.overlay));
            close_modal(&fixture);

            wait_until(|| focused_widget_shows(&fixture.window, "folder"));
            assert_eq!(
                (focused_depth(&browser), focused_name(&browser)),
                (Some(0), "folder".to_owned())
            );
        },
    );
}

#[test]
fn tenxer_delete_targets_the_open_folder_when_the_child_selects_nothing() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::delete::tenxer_delete_targets_the_open_folder_when_the_child_selects_nothing",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            seed_folder(&fixture, "folder");
            activate_folder(&fixture, "folder");

            plain(&fixture, Key::d);
            wait_until(|| modal_visible(&fixture.overlay));
            wait_confirm_focused(&fixture);
            assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
            wait_until(|| !directory.join("folder").exists());
            assert_eq!(directory_names(&directory), ["a.txt", "b.txt", "c.txt"]);
        },
    );
}

/// With a committed search or filter that matched nothing, the delete keys
/// report nothing to delete rather than targeting the open folder.
fn empty_hits_delete_nothing(fixture: &KeyboardFixture, directory: &Path, search: bool) {
    let folder = directory.join("folder");
    if search {
        fixture.view.commit_listing_search("no-match");
    } else {
        fixture.view.commit_listing_filter("no-match");
    }
    wait_until(|| fixture.view.selected_search_results() == Some(Vec::new()));
    focus_files(fixture);
    for (key, modifiers) in [
        (Key::d, ModifierType::empty()),
        (Key::D, ModifierType::SHIFT_MASK),
    ] {
        assert!(fixture.press(key, modifiers));
        assert_eq!(feedback(fixture), "Nothing to delete");
        assert!(!modal_visible(&fixture.overlay));
        let kept = (folder.is_dir(), directory_names(directory));
        assert_eq!(
            kept,
            (
                true,
                ["a.txt", "b.txt", "c.txt", "folder"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
    }
    if search {
        assert!(fixture.view.dismiss_listing_search());
    } else {
        assert!(fixture.view.clear_listing_filter());
    }
    focus_files(fixture);
}

#[test]
fn tenxer_delete_with_no_search_or_filter_hits_preserves_the_open_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::delete::tenxer_delete_with_no_search_or_filter_hits_preserves_the_open_folder",
        || {
            let fixture = KeyboardFixture::new();
            enable_tenxer(&fixture);
            let directory = fixture._directory.path().to_path_buf();
            seed_folder(&fixture, "folder");
            enter_folder(&fixture, "folder");

            for search in [false, true] {
                empty_hits_delete_nothing(&fixture, &directory, search);
            }
        },
    );
}
