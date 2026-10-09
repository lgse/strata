// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::{BrowserView, PeekBehavior};

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn find_widget<T: IsA<gtk::Widget> + glib::object::IsClass>(
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

pub(super) fn button(root: &gtk::Widget, name: &str) -> Option<gtk::Button> {
    find_widget(root, &|button: &gtk::Button| {
        button.label().as_deref() == Some(name) && button.is_visible() && button.is_sensitive()
    })
}

pub(super) fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        if Instant::now() >= deadline {
            for window in gtk::Window::list_toplevels() {
                find_widget(&window, &|label: &gtk::Label| {
                    eprintln!("label: {}", label.text());
                    false
                });
            }
            panic!("operation timed out");
        }
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn view() -> BrowserView {
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view
}

pub(super) fn window(view: &BrowserView) -> gtk::Window {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(1000)
        .default_height(650)
        .build();
    window.present();
    window
}

fn trashed_entry(root: &Path, name: &str, original: &Path) -> (FileEntry, PathBuf) {
    let trash = root.join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
    fs::create_dir_all(trash.join("files")).expect("files");
    fs::create_dir_all(trash.join("info")).expect("info");
    let physical = trash.join("files").join(name);
    fs::write(&physical, b"original").expect("payload");
    let info = trash.join("info").join(format!("{name}.trashinfo"));
    fs::write(
        &info,
        format!("[Trash Info]\nPath={}\n", original.display()),
    )
    .expect("metadata");
    (
        FileEntry {
            location: Location::uri(format!("trash:///{name}")),
            thumbnail_path: Some(physical),
            native_name: name.into(),
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
            recent_uri: None,
        },
        info,
    )
}

#[test]
fn permanent_delete_confirmation_is_actionable_while_summary_loads() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::permanent_delete_confirmation_is_actionable_while_summary_loads",
        || {
            for (locale, confirm_text, cancel_text, warning) in [
                (
                    "en",
                    "Permanently delete 1 item",
                    "Cancel",
                    "This item will be permanently deleted. This action cannot be undone.",
                ),
                (
                    "fr",
                    "Supprimer définitivement 1 élément",
                    "Annuler",
                    "Cet élément sera supprimé définitivement. Cette action est irréversible.",
                ),
            ] {
                let fixture = tempfile::tempdir().expect("fixture");
                let original = fixture.path().join("original.txt");
                let (entry, _) = trashed_entry(fixture.path(), "Language", &original);
                let physical = entry.thumbnail_path.clone().expect("physical trash file");
                let view = view();
                let window = window(&view);
                rust_i18n::set_locale(locale);
                view.state.show_delete_confirmation(vec![entry]);

                let confirm = find_widget(window.upcast_ref(), &|button: &gtk::Button| {
                    button.label().as_deref() == Some(confirm_text)
                })
                .expect("permanent delete confirmation");
                assert!(confirm.is_sensitive());
                assert!(
                    find_widget(window.upcast_ref(), &|label: &gtk::Label| {
                        label
                            .text()
                            .split_whitespace()
                            .eq(warning.split_whitespace())
                    })
                    .is_some(),
                    "localized consequence warning: {locale}"
                );
                assert!(
                    find_widget(window.upcast_ref(), &|label: &gtk::Label| label.text()
                        == "Language")
                    .is_some(),
                    "filename must not become a translation key"
                );
                button(window.upcast_ref(), cancel_text)
                    .expect("localized cancel")
                    .emit_clicked();
                assert!(physical.exists(), "cancelling must preserve the file");
                window.close();
            }
        },
    );
}

#[test]
fn changing_metadata_after_confirmation_is_presented_refuses_the_move() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::changing_metadata_after_confirmation_is_presented_refuses_the_move",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let confirmed = fixture.path().join("confirmed");
            let changed = fixture.path().join("changed");
            let (entry, info) = trashed_entry(fixture.path(), "safe", &confirmed);
            let source = entry.thumbnail_path.clone().expect("source");
            let view = view();
            let window = window(&view);
            view.state.request_restore(vec![entry]);
            wait_until(|| button(&window.clone().upcast(), "Restore").is_some());
            fs::write(&info, format!("[Trash Info]\nPath={}\n", changed.display()))
                .expect("change metadata");
            button(&window.clone().upcast(), "Restore")
                .expect("confirm")
                .emit_clicked();
            wait_until(|| {
                find_widget(&window.clone().upcast(), &|label: &gtk::Label| {
                    label
                        .text()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .contains("no longer matches the confirmed destination")
                })
                .is_some()
            });
            assert!(source.exists());
            assert!(info.exists());
            assert!(!confirmed.exists());
            assert!(!changed.exists());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

fn normalized(label: &gtk::Label) -> String {
    label
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_label(root: &gtk::Window, text: &str) -> bool {
    find_widget(root.upcast_ref(), &|label: &gtk::Label| {
        normalized(label).contains(text)
    })
    .is_some()
}

fn focused_button_label(window: &gtk::Window) -> Option<String> {
    gtk::prelude::RootExt::focus(window)?
        .downcast::<gtk::Button>()
        .ok()?
        .label()
        .map(String::from)
}

fn delete_summary_loaded(window: &gtk::Window) -> bool {
    find_widget(window.upcast_ref(), &|label: &gtk::Label| {
        let text = normalized(label);
        text.contains(" · ") && text.ends_with("will be permanently deleted")
    })
    .is_some()
}

/// Window dispatch cannot reach the modal's own key controller.
fn modal_key(window: &gtk::Window, key: gtk::gdk::Key) -> bool {
    let layer = find_widget(window.upcast_ref(), &|widget: &gtk::Widget| {
        widget.has_css_class("app-modal-layer")
    })
    .expect("open dialog");
    let controllers = layer.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("dialog key controller")
        .emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
        )
}

fn pump(milliseconds: u64) {
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn unsupported_trash_fallback_explains_and_focuses_cancel() {
    crate::test_support::gtk_test(
        "ui::browser::trash::tests::unsupported_trash_fallback_explains_and_focuses_cancel",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let paths = ["a.txt", "b.txt"].map(|name| fixture.path().join(name));
            for path in &paths {
                fs::write(path, b"keep").expect("fixture file");
            }
            let entries: Vec<FileEntry> = paths
                .iter()
                .map(|path| crate::test_support::operations::entry(Location::local(path)))
                .collect();
            let held = Rc::new(crate::test_support::operations::HeldOperations::default());
            let view = view();
            view.set_operation_provider(held.clone());
            let window = window(&view);

            // `mixed`: another item failed for a different reason, so the
            // "Completed with errors" dialog's Delete Permanently comes first.
            for mixed in [false, true] {
                let unsupported = |id| {
                    crate::services::OperationEvent::CompletedWithErrors {
                    request_id: id,
                    deleted_locations: Vec::new(),
                    retryable_locations: entries.iter().map(|entry| entry.location.clone()).collect(),
                    has_non_retryable_failures: mixed,
                    message: "a.txt: This location doesn't support Trash. Delete permanently instead.\n\n• b.txt: This location doesn't support Trash. Delete permanently instead.".into(),
                }
                };
                let fail_trash_attempt = || {
                    view.state.request_delete(entries.clone(), false);
                    let id = view
                        .browser()
                        .last_started_operation()
                        .expect("trash attempt");
                    assert_eq!(
                        held.delete_requests.borrow().last().copied(),
                        Some((id, false, 2)),
                        "Delete first attempts a trash"
                    );
                    held.emit(id, unsupported(id));
                    if mixed {
                        wait_until(|| button(window.upcast_ref(), "Delete Permanently").is_some());
                        wait_until(|| {
                            focused_button_label(&window).as_deref() == Some("Delete Permanently")
                        });
                        button(window.upcast_ref(), "Delete Permanently")
                            .expect("retry")
                            .emit_clicked();
                    }
                    wait_until(|| {
                        button(window.upcast_ref(), "Permanently delete 2 items").is_some()
                    });
                };

                fail_trash_attempt();
                wait_until(|| {
                    focused_button_label(&window).is_some_and(|label| {
                        label == "Cancel" || label == "Permanently delete 2 items"
                    })
                });
                let initial_focus = focused_button_label(&window);
                wait_until(|| delete_summary_loaded(&window));
                pump(300);
                assert_eq!(
                    (
                        has_label(
                            &window,
                            "doesn't support Trash. These items will be permanently deleted."
                        ),
                        initial_focus.as_deref(),
                        focused_button_label(&window).as_deref(),
                    ),
                    (true, Some("Cancel"), Some("Cancel")),
                    "mixed: {mixed}: (explains missing Trash, initial focus, focus after the size summary)"
                );

                let requests = held.delete_requests.borrow().len();
                assert!(modal_key(&window, gtk::gdk::Key::Return));
                wait_until(|| button(window.upcast_ref(), "Permanently delete 2 items").is_none());
                assert_eq!(
                    held.delete_requests.borrow().len(),
                    requests,
                    "Enter cancels"
                );
                assert!(paths.iter().all(|path| path.exists()));

                fail_trash_attempt();
                button(window.upcast_ref(), "Permanently delete 2 items")
                    .expect("confirm")
                    .emit_clicked();
                wait_until(|| held.delete_requests.borrow().len() == requests + 2);
                let (id, permanent, count) = held
                    .delete_requests
                    .borrow()
                    .last()
                    .copied()
                    .expect("retry");
                assert_eq!(
                    (permanent, count),
                    (true, 2),
                    "mixed: {mixed}: confirm is permanent"
                );
                held.emit(
                    id,
                    crate::services::OperationEvent::Deleted {
                        request_id: id,
                        locations: entries.iter().map(|entry| entry.location.clone()).collect(),
                    },
                );
                wait_until(|| !view.browser().is_current_operation(id));
            }
            window.destroy();
            view.browser().clear_observer();
        },
    );
}
