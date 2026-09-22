// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{FileEntry, Location};
use crate::ui::browser::{BrowserView, PeekBehavior};
use gtk::glib;
use gtk::prelude::{GtkWindowExt, IsA};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[test]
fn delete_confirmation_direction_keys_choose_an_action() {
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::Left),
        Some(DeleteConfirmationFocus::Cancel)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::h),
        Some(DeleteConfirmationFocus::Cancel)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::Right),
        Some(DeleteConfirmationFocus::Confirm)
    );
    assert_eq!(
        delete_confirmation_focus_target(gtk::gdk::Key::l),
        Some(DeleteConfirmationFocus::Confirm)
    );
    assert_eq!(delete_confirmation_focus_target(gtk::gdk::Key::Tab), None);
}

#[test]
fn retryable_delete_entries_keeps_only_the_named_locations() {
    let entry = |name: &str| FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        thumbnail_path: None,
        display_name: name.into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    };
    let retryable = entry("share-file.txt");
    let denied = entry("locked-file.txt");
    let entries = vec![retryable.clone(), denied];

    let kept = retryable_delete_entries(entries, std::slice::from_ref(&retryable.location));

    assert_eq!(kept, vec![retryable]);
}

#[test]
fn retryable_delete_entries_is_empty_when_nothing_matches() {
    let entry = FileEntry {
        location: Location::local("/fixture/photo"),
        native_name: "photo".into(),
        thumbnail_path: None,
        display_name: "photo".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    };

    let kept = retryable_delete_entries(vec![entry], &[]);

    assert!(kept.is_empty());
}

#[test]
fn restore_confirmation_shows_the_full_destination_path() {
    assert_eq!(
        restore_destination_text(std::path::Path::new(
            "/home/user/Documents/Projects/report.txt"
        )),
        "/home/user/Documents/Projects/report.txt"
    );
}

#[test]
fn restore_confirmation_names_the_item_count_and_destination_action() {
    assert_eq!(restore_confirmation_title(1), "Restore 1 item?");
    assert_eq!(restore_confirmation_title(3), "Restore 3 items?");
    assert_eq!(restore_confirmation_confirm_label(1), "Restore");
    assert_eq!(restore_confirmation_confirm_label(2), "Restore 2 items");
}

#[test]
fn restore_error_summary_includes_the_failure_reason() {
    assert_eq!(
        restore_error_summary(&[
            "notes.txt: The original location is outside the trash volume and cannot be restored"
                .to_owned()
        ]),
        "notes.txt: The original location is outside the trash volume and cannot be restored"
    );
    let summary = restore_error_summary(&["a: denied".to_owned(), "b: denied".to_owned()]);
    assert!(summary.starts_with("2 items could not be restored."));
    assert!(summary.contains("a: denied"));
}

#[test]
fn delete_confirmation_renders_every_row_for_a_small_selection() {
    let entries = (0..7).map(confirmation_entry).collect::<Vec<_>>();

    let (visible, hidden) = delete_confirmation_rows(&entries);

    assert_eq!(visible.len(), 7);
    assert_eq!(hidden, 0);
    assert_eq!(delete_confirmation_overflow_label(hidden), None);
}

#[test]
fn delete_confirmation_caps_rows_and_summarizes_the_rest() {
    let entries = (0..1000).map(confirmation_entry).collect::<Vec<_>>();

    let (visible, hidden) = delete_confirmation_rows(&entries);

    assert_eq!(visible.len(), DELETE_CONFIRMATION_MAX_ROWS);
    assert_eq!(hidden, 1000 - DELETE_CONFIRMATION_MAX_ROWS);
    assert_eq!(
        delete_confirmation_overflow_label(hidden),
        Some("… and 950 more items".to_owned())
    );
}

#[test]
fn delete_confirmation_overflow_label_uses_the_singular_for_one_item() {
    assert_eq!(
        delete_confirmation_overflow_label(1),
        Some("… and 1 more item".to_owned())
    );
}

fn confirmation_entry(index: usize) -> FileEntry {
    let name = format!("file-{index}.txt");
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.clone().into(),
        thumbnail_path: None,
        display_name: name,
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    }
}

struct DeleteConfirmation {
    view: BrowserView,
    window: gtk::Window,
    path: PathBuf,
    layer: gtk::Widget,
    keys: Vec<gtk::EventControllerKey>,
    cancel: gtk::Button,
    confirm: gtk::Button,
    close: gtk::Button,
}

impl DeleteConfirmation {
    fn present() -> (tempfile::TempDir, Self) {
        let fixture = tempfile::tempdir().expect("fixture");
        let path = fixture.path().join("keep-me.txt");
        std::fs::write(&path, b"payload").expect("temp file");
        let view = BrowserView::new(
            Rc::new(crate::adapters::LocalFileSource),
            PeekBehavior::default(),
        );
        view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&view.widget()));
        let window = gtk::Window::builder()
            .child(&overlay)
            .default_width(1000)
            .default_height(650)
            .build();
        window.present();
        view.state
            .show_delete_confirmation(vec![local_file_entry(&path)], true, false);
        let root = window.clone().upcast::<gtk::Widget>();
        wait_until(
            || {
                button(&root, |button| {
                    button.label().as_deref() == Some(CONFIRM_LABEL)
                })
                .is_some()
            },
            "delete confirmation should appear",
        );
        let confirm = button(&root, |button| {
            button.label().as_deref() == Some(CONFIRM_LABEL)
        })
        .expect("confirm");
        let cancel =
            button(&root, |button| button.label().as_deref() == Some("Cancel")).expect("cancel");
        let close = button(&root, |button| {
            button.tooltip_text().as_deref() == Some("Close dialog")
        })
        .expect("close");
        wait_until(|| confirm.has_focus(), "confirm should take initial focus");
        let layer = find_widget(&root, &|widget: &gtk::Widget| {
            widget.has_css_class("app-modal-layer")
        })
        .expect("modal layer");
        let keys = key_controllers(&layer);
        (
            fixture,
            Self {
                view,
                window,
                path,
                layer,
                keys,
                cancel,
                confirm,
                close,
            },
        )
    }

    fn press(&self, key: gtk::gdk::Key) {
        assert!(press(&self.keys, key), "modal layer should handle {key:?}");
    }

    fn dismissed(&self) -> bool {
        self.layer.parent().is_none()
    }

    fn finish(self) {
        self.window.destroy();
        self.view.browser().clear_observer();
    }
}

const CONFIRM_LABEL: &str = "Permanently delete 1 item";

#[test]
fn enter_keeps_file_on_cancel() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::enter_keeps_file_on_cancel",
        || {
            let (_dir, dialog) = DeleteConfirmation::present();
            dialog.press(gtk::gdk::Key::Left);
            wait_until(
                || dialog.cancel.has_focus() && !dialog.confirm.has_focus(),
                "Left should move focus to Cancel",
            );
            dialog.press(gtk::gdk::Key::Return);
            wait_until(
                || dialog.dismissed(),
                "Cancel-focused Enter should dismiss the dialog",
            );
            drain_past_dismiss_timeout();
            assert!(
                dialog.path.exists(),
                "Cancel-focused Enter should not delete the file"
            );
            dialog.finish();
        },
    );
}

#[test]
fn enter_deletes_on_confirm() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::enter_deletes_on_confirm",
        || {
            let (dir, dialog) = DeleteConfirmation::present();
            assert!(
                dialog.confirm.has_focus(),
                "confirm should keep initial focus"
            );
            dialog.press(gtk::gdk::Key::Return);
            wait_until(
                || !dialog.path.exists(),
                "confirm-focused Enter should permanently delete the file",
            );
            wait_until(
                || dialog.dismissed(),
                "confirm-focused Enter should dismiss the dialog",
            );
            assert!(
                !trashed_copy_exists(dir.path(), "keep-me.txt"),
                "permanent delete should not leave the file in Trash"
            );
            dialog.finish();
        },
    );
}

#[test]
fn enter_keeps_file_on_close() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::enter_keeps_file_on_close",
        || {
            let (_dir, dialog) = DeleteConfirmation::present();
            assert!(
                dialog.close.grab_focus(),
                "Close should take focus before Enter"
            );
            assert!(
                dialog.close.has_focus() && !dialog.confirm.has_focus(),
                "Close should own focus before Enter"
            );
            dialog.press(gtk::gdk::Key::Return);
            wait_until(
                || dialog.dismissed(),
                "Close-focused Enter should dismiss the dialog",
            );
            drain_past_dismiss_timeout();
            assert!(
                dialog.path.exists(),
                "Close-focused Enter should not delete the file"
            );
            dialog.finish();
        },
    );
}

fn local_file_entry(path: &Path) -> FileEntry {
    let name = path
        .file_name()
        .expect("should have a file name")
        .to_os_string();
    FileEntry {
        location: Location::local(path),
        native_name: name.clone(),
        thumbnail_path: None,
        display_name: name.to_string_lossy().into_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
    }
}

fn find_widget<T: IsA<gtk::Widget> + glib::object::IsClass>(
    root: &gtk::Widget,
    predicate: &impl Fn(&T) -> bool,
) -> Option<T> {
    if let Some(widget) = root.downcast_ref::<T>()
        && predicate(widget)
    {
        return Some(widget.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find_widget(&widget, predicate) {
            return Some(found);
        }
    }
    None
}

fn button(root: &gtk::Widget, predicate: impl Fn(&gtk::Button) -> bool) -> Option<gtk::Button> {
    find_widget(root, &|button: &gtk::Button| {
        button.is_visible() && button.is_sensitive() && predicate(button)
    })
}

fn key_controllers(widget: &impl IsA<gtk::Widget>) -> Vec<gtk::EventControllerKey> {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .collect()
}

fn press(keys: &[gtk::EventControllerKey], key: gtk::gdk::Key) -> bool {
    // GTK prepends controllers, so observe order is last-added first.
    keys.iter().any(|controller| {
        controller.emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
        )
    })
}

fn drain_past_dismiss_timeout() {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn wait_until(condition: impl Fn() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn trashed_copy_exists(root: &Path, name: &str) -> bool {
    let mut stack = vec![root.to_path_buf()];
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        stack.push(PathBuf::from(data_home).join("Trash/files"));
    }
    if let Ok(home) = std::env::var("HOME") {
        stack.push(PathBuf::from(home).join(".local/share/Trash/files"));
    }
    stack.into_iter().any(|dir| dir.join(name).exists())
}
