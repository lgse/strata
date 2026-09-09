// SPDX-License-Identifier: MIT

use super::*;

mod confirmation;
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
            is_hidden: false,
            mode: crate::model::MetadataValue::Unknown,
        },
        info,
    )
}

#[test]
fn restore_confirms_full_destination_and_skips_invalid_items() {
    crate::test_support::gtk_test(
        "ui::browser::tests::restore::restore_confirms_full_destination_and_skips_invalid_items",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let destination = fixture.path().join("restored");
            let (entry, info) = trashed_entry(fixture.path(), "safe", &destination);
            let (bad, bad_info) = trashed_entry(
                fixture.path(),
                "unsafe",
                Path::new("/proc/self/strata-restore-reject"),
            );
            let source = entry.thumbnail_path.clone().expect("source");
            let bad_source = bad.thumbnail_path.clone().expect("bad source");
            let view = view();
            let window = window(&view);
            view.state.request_restore(vec![entry, bad]);
            wait_until(|| button(&window.clone().upcast(), "Restore").is_some());
            assert!(!destination.exists(), "lookup must not move anything");
            let scroller = find_widget(
                &window.clone().upcast(),
                &|scroller: &gtk::ScrolledWindow| {
                    scroller.has_css_class("delete-confirmation-list")
                },
            )
            .expect("destination list");
            assert_eq!(scroller.vscrollbar_policy(), gtk::PolicyType::Automatic);
            assert_eq!(scroller.max_content_height(), 256);
            assert!(
                find_widget(&window.clone().upcast(), &|label: &gtk::Label| label
                    .text()
                    .as_str()
                    == destination.to_str().expect("path"))
                .is_some()
            );
            assert!(
                find_widget(&window.clone().upcast(), &|label: &gtk::Label| label
                    .text()
                    .contains("unsafe:"))
                .is_some()
            );
            button(&window.clone().upcast(), "Restore")
                .expect("confirm")
                .emit_clicked();
            wait_until(|| destination.exists() && !info.exists());
            assert_eq!(fs::read(&destination).expect("restored"), b"original");
            assert!(!source.exists());
            assert!(bad_source.exists());
            assert!(bad_info.exists());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn cancelling_restore_lookup_keeps_payload_and_metadata() {
    crate::test_support::gtk_test(
        "ui::browser::tests::restore::cancelling_restore_lookup_keeps_payload_and_metadata",
        || {
            for close in [false, true] {
                let fixture = tempfile::tempdir().expect("fixture");
                let destination = fixture.path().join("restored");
                let (entry, info) = trashed_entry(fixture.path(), "safe", &destination);
                let source = entry.thumbnail_path.clone().expect("source");
                let view = view();
                let window = window(&view);
                view.state.request_restore(vec![entry]);
                assert!(view.state.pending_trash_lookup.borrow().is_some());
                let cancel = if close {
                    find_widget(&window.clone().upcast(), &|button: &gtk::Button| {
                        button.tooltip_text().as_deref() == Some("Close dialog")
                            && button.is_sensitive()
                    })
                } else {
                    button(&window.clone().upcast(), "Cancel")
                };
                cancel.expect("cancel lookup").emit_clicked();
                assert!(view.state.pending_trash_lookup.borrow().is_none());
                assert!(view.state.trash_loading.borrow().is_none());
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                assert!(source.exists());
                assert!(info.exists());
                assert!(!destination.exists());
                assert!(button(&window.clone().upcast(), "Restore").is_none());
                window.destroy();
                view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn missing_window_host_reports_an_error_without_restoring() {
    crate::test_support::gtk_test(
        "ui::browser::tests::restore::missing_window_host_reports_an_error_without_restoring",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let destination = fixture.path().join("restored");
            let (entry, info) = trashed_entry(fixture.path(), "safe", &destination);
            let source = entry.thumbnail_path.clone().expect("source");
            let view = view();
            view.state.request_restore(vec![entry]);
            assert!(view.state.pending_trash_lookup.borrow().is_none());
            assert!(
                find_widget(&view.widget(), &|label: &gtk::Label| label.text()
                    == "Unable to continue")
                .is_some()
            );
            assert!(source.exists());
            assert!(info.exists());
            assert!(!destination.exists());
            view.browser().clear_observer();
        },
    );
}
