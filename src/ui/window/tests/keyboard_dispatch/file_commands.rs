// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::*;
use crate::test_support::operations::HeldOperations;
use crate::ui::tenxer_mode::Prompt;

mod after_back;
mod archives;
mod clipboard;
mod create;
mod delete;
mod send_to;
mod trashless;

fn held_fixture(
    held: &Rc<HeldOperations>,
    source: Rc<dyn crate::services::FileSource>,
) -> KeyboardFixture {
    let provider = held.clone();
    KeyboardFixture::with_parts(Rc::new(TextPreview), move || {
        let view = BrowserView::new(source, crate::ui::browser::PeekBehavior::default());
        view.set_operation_provider(provider);
        view
    })
}

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
    keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &crate::ui::keys::gdk_key(key),
            &0u32,
            &crate::ui::keys::gdk_modifiers(ModifierType::empty()),
        ],
    )
}

fn confirm_button(fixture: &KeyboardFixture) -> Option<gtk::Widget> {
    widget_with_class(fixture.overlay.upcast_ref(), "action-dialog-confirm")
}

fn wait_confirm_sensitive(fixture: &KeyboardFixture) {
    wait_until(|| confirm_button(fixture).is_some_and(|confirm| confirm.is_sensitive()));
}

fn wait_confirm_focused(fixture: &KeyboardFixture) {
    wait_until(|| {
        confirm_button(fixture).is_some_and(|confirm| confirm.is_sensitive() && confirm.has_focus())
    });
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

/// Creates `folder` in the fixture root and refreshes until it is listed.
fn seed_folder(fixture: &KeyboardFixture, name: &str) -> PathBuf {
    let folder = fixture._directory.path().join(name);
    std::fs::create_dir(&folder).expect("fixture folder");
    fixture.view.browser().refresh_all();
    wait_until(|| rendered_name(&fixture.view.widget(), name));
    folder
}

/// Activates the listed folder `name` in the first column and focuses its child.
fn activate_folder(fixture: &KeyboardFixture, name: &str) {
    let browser = fixture.view.browser();
    let count = browser.column_snapshot(0).expect("column").count;
    let folder_position = browser
        .with_entries(0, 0..count, |entries| {
            entries.iter().position(|entry| entry.display_name == name)
        })
        .flatten()
        .expect("folder is listed");
    browser.activate(0, folder_position);
    wait_loaded(&browser, 1);
    focus_files(fixture);
}

fn undo(fixture: &KeyboardFixture) {
    wait_until(|| fixture.press(Key::z, ModifierType::CONTROL_MASK));
}
