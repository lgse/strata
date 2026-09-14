// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    app::Browser,
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest},
};

use super::{PreviewPopup, preview_target};

struct NoopPreviewProvider;

impl PreviewProvider for NoopPreviewProvider {
    fn load(&self, _request: PreviewRequest, _emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

fn file_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/tmp/{name}")),
        thumbnail_path: None,
        native_name: std::ffi::OsString::from(name),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn popup_opens_and_closes_on_toggle() {
    const TEST: &str = "ui::preview::popup::tests::popup_opens_and_closes_on_toggle";
    crate::test_support::gtk_test(TEST, || {
        let app = gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::FLAGS_NONE);
        let window = gtk::ApplicationWindow::new(&app);
        let popup = PreviewPopup::new(Rc::new(NoopPreviewProvider), &window);
        let entry = file_entry("photo.png");

        popup.toggle(preview_target(Some(entry.clone())), Some(0));
        assert!(popup.is_open());

        popup.toggle(preview_target(Some(entry)), Some(0));
        assert!(!popup.is_open());
    });
}

#[test]
fn popup_close_disables_follow_selection() {
    const TEST: &str = "ui::preview::popup::tests::popup_close_disables_follow_selection";
    crate::test_support::gtk_test(TEST, || {
        let app = gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::FLAGS_NONE);
        let window = gtk::ApplicationWindow::new(&app);
        let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
        let popup = PreviewPopup::new(Rc::new(NoopPreviewProvider), &window);
        popup.observe_browser(&browser);
        let entry = file_entry("photo.png");

        popup.open(preview_target(Some(entry)), Some(0));
        assert!(popup.is_open());
        popup.close();
        assert!(!popup.is_open());
    });
}
