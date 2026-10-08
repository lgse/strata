// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;

fn plain(fixture: &KeyboardFixture, key: Key) {
    assert!(fixture.press(key, ModifierType::empty()), "{key:?}");
}

fn enable_tree_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
    fixture.view.browser().clear_active_selection();
}

fn tree_focused_name(view: &BrowserView) -> Option<String> {
    view.tree_focused_entry().map(|entry| entry.display_name)
}

fn make_nested(fixture: &KeyboardFixture) -> std::path::PathBuf {
    let sub = fixture._directory.path().join("sub");
    std::fs::create_dir(&sub).expect("tree fixture folder");
    std::fs::write(sub.join("inner.txt"), b"inner").expect("tree fixture file");
    fixture.view.refresh();
    wait_until(|| rendered_name(&fixture.view.widget(), "sub"));
    wait_until(|| !rendered_name(&fixture.view.widget(), "inner.txt"));
    sub
}

fn close_modal(fixture: &KeyboardFixture) {
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
}

fn focus_tree(fixture: &KeyboardFixture) {
    fixture.view.set_view_mode(BrowserMode::Tree);
    focus_files(fixture);
}

#[test]
fn tree_shows_root_rows() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_shows_root_rows",
        || {
            let fixture = KeyboardFixture::new();
            focus_tree(&fixture);
            wait_until(|| rendered_name(&fixture.view.widget(), "a.txt"));
            assert!(rendered_name(&fixture.view.widget(), "b.txt"));
            assert!(rendered_name(&fixture.view.widget(), "c.txt"));
        },
    );
}

#[test]
fn tree_expands_and_collapses_branches() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_expands_and_collapses_branches",
        || {
            let fixture = KeyboardFixture::new();
            make_nested(&fixture);
            focus_tree(&fixture);
            // Root focus starts on the first row; reach the folder by name.
            plain(&fixture, Key::Home);
            while tree_focused_name(&fixture.view).as_deref() != Some("sub") {
                plain(&fixture, Key::Down);
            }
            plain(&fixture, Key::Right);
            wait_until(|| rendered_name(&fixture.view.widget(), "inner.txt"));
            assert_eq!(
                tree_focused_name(&fixture.view).as_deref(),
                Some("sub"),
                "expanding keeps the cursor on the folder"
            );
            plain(&fixture, Key::Left);
            wait_until(|| !rendered_name(&fixture.view.widget(), "inner.txt"));
        },
    );
}

#[test]
fn tree_arrow_down_steps_into_expanded_children() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_arrow_down_steps_into_expanded_children",
        || {
            let fixture = KeyboardFixture::new();
            make_nested(&fixture);
            focus_tree(&fixture);
            plain(&fixture, Key::Home);
            while tree_focused_name(&fixture.view).as_deref() != Some("sub") {
                plain(&fixture, Key::Down);
            }
            plain(&fixture, Key::Right);
            wait_until(|| rendered_name(&fixture.view.widget(), "inner.txt"));
            plain(&fixture, Key::Down);
            assert_eq!(
                tree_focused_name(&fixture.view).as_deref(),
                Some("inner.txt")
            );
            // Left on a collapsed child climbs to the parent row.
            plain(&fixture, Key::Left);
            assert_eq!(tree_focused_name(&fixture.view).as_deref(), Some("sub"));
        },
    );
}

#[test]
fn tree_tenxer_verbs_resolve_nested_rows() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_tenxer_verbs_resolve_nested_rows",
        || {
            let fixture = KeyboardFixture::new();
            make_nested(&fixture);
            enable_tree_tenxer(&fixture);
            focus_tree(&fixture);
            plain(&fixture, Key::Home);
            while tree_focused_name(&fixture.view).as_deref() != Some("sub") {
                plain(&fixture, Key::j);
            }
            // l expands in tree mode instead of opening the folder.
            plain(&fixture, Key::l);
            wait_until(|| rendered_name(&fixture.view.widget(), "inner.txt"));
            assert_eq!(
                fixture.view.browser().active_depth(),
                Some(0),
                "expanding never descends the column model"
            );
            plain(&fixture, Key::j);
            assert_eq!(
                tree_focused_name(&fixture.view).as_deref(),
                Some("inner.txt")
            );
            let targets = fixture.view.command_targets();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].display_name, "inner.txt");
            assert_eq!(
                fixture
                    .view
                    .focused_target()
                    .map(|entry| entry.display_name),
                Some("inner.txt".to_owned())
            );
        },
    );
}

#[test]
fn tree_tenxer_mark_and_yank_cover_nested_selection() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_tenxer_mark_and_yank_cover_nested_selection",
        || {
            let fixture = KeyboardFixture::new();
            make_nested(&fixture);
            enable_tree_tenxer(&fixture);
            focus_tree(&fixture);
            plain(&fixture, Key::Home);
            while tree_focused_name(&fixture.view).as_deref() != Some("sub") {
                plain(&fixture, Key::j);
            }
            plain(&fixture, Key::l);
            wait_until(|| rendered_name(&fixture.view.widget(), "inner.txt"));
            plain(&fixture, Key::j);
            // Space marks the nested cursor row like the column cursor mark.
            plain(&fixture, Key::space);
            let targets = fixture.view.command_targets();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].display_name, "inner.txt");
            fixture.view.yank_targets(false);
            assert_eq!(
                crate::ui::browser::clipboard_mark(&targets[0].location),
                crate::ui::browser::ClipboardMark::Copy
            );
            // Escape clears tree marks; the column selection was already empty.
            plain(&fixture, Key::Escape);
            assert!(fixture.view.command_targets().is_empty());
        },
    );
}

#[test]
fn tree_delete_targets_the_nested_cursor() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_delete_targets_the_nested_cursor",
        || {
            let fixture = KeyboardFixture::new();
            let sub = make_nested(&fixture);
            enable_tree_tenxer(&fixture);
            focus_tree(&fixture);
            plain(&fixture, Key::Home);
            while tree_focused_name(&fixture.view).as_deref() != Some("sub") {
                plain(&fixture, Key::j);
            }
            plain(&fixture, Key::l);
            wait_until(|| rendered_name(&fixture.view.widget(), "inner.txt"));
            plain(&fixture, Key::j);
            assert!(fixture.view.confirm_delete(true));
            assert!(modal_visible(&fixture.overlay), "permanent delete confirms");
            close_modal(&fixture);
            assert!(sub.join("inner.txt").exists(), "cancel keeps the file");
        },
    );
}

#[test]
fn tree_rename_opens_the_editor_on_nested_rows() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::tree_view::tree_rename_opens_the_editor_on_nested_rows",
        || {
            let fixture = KeyboardFixture::new();
            make_nested(&fixture);
            enable_tree_tenxer(&fixture);
            focus_tree(&fixture);
            plain(&fixture, Key::Home);
            while tree_focused_name(&fixture.view).as_deref() != Some("sub") {
                plain(&fixture, Key::j);
            }
            plain(&fixture, Key::l);
            wait_until(|| rendered_name(&fixture.view.widget(), "inner.txt"));
            plain(&fixture, Key::j);
            assert!(fixture.view.begin_rename());
            wait_until(|| fixture.view.rename_is_active());
            plain(&fixture, Key::Escape);
            wait_until(|| !fixture.view.rename_is_active());
        },
    );
}
