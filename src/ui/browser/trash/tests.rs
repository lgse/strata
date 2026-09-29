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
        },
        info,
    )
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
