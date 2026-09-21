// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    app::Browser,
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
        PreviewEvent, PreviewProvider, PreviewRequest,
    },
};

use super::{PreviewDrawer, PreviewPopup, preview_target};

struct NoopPreviewProvider;

impl PreviewProvider for NoopPreviewProvider {
    fn load(&self, _request: PreviewRequest, _emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

struct TestSource {
    entries: Vec<FileEntry>,
}

impl FileSource for TestSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: self.entries.clone(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
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

fn popup_fixture(window: &gtk::ApplicationWindow) -> (PreviewDrawer, PreviewPopup) {
    let drawer = PreviewDrawer::new(Rc::new(NoopPreviewProvider), false);
    let popup = PreviewPopup::new(window, drawer.clone());
    (drawer, popup)
}

fn application() -> gtk::Application {
    if let Some(application) = gtk::gio::Application::default().and_downcast::<gtk::Application>() {
        return application;
    }
    let application = gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gtk::gio::Cancellable>)
        .expect("test application registration");
    application
}

#[test]
fn popup_floats_the_shared_session_and_docks_it_back_on_close() {
    const TEST: &str =
        "ui::preview::popup::tests::popup_floats_the_shared_session_and_docks_it_back_on_close";
    crate::test_support::gtk_test(TEST, || {
        let app = application();
        let window = gtk::ApplicationWindow::new(&app);
        let (drawer, popup) = popup_fixture(&window);
        let entry = file_entry("photo.png");

        popup.toggle(preview_target(Some(entry.clone())), Some(0));
        assert!(popup.is_open());
        assert!(drawer.is_floating());

        popup.toggle(preview_target(Some(entry)), Some(0));
        assert!(!popup.is_open());
        assert!(!drawer.is_floating());
        assert!(!drawer.is_enabled());
    });
}

#[test]
fn popup_restores_a_previously_open_pane_on_close() {
    const TEST: &str = "ui::preview::popup::tests::popup_restores_a_previously_open_pane_on_close";
    crate::test_support::gtk_test(TEST, || {
        let app = application();
        let window = gtk::ApplicationWindow::new(&app);
        let (drawer, popup) = popup_fixture(&window);
        let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
        drawer.observe_browser(&browser);
        popup.observe_browser(&browser);
        let entry = file_entry("photo.png");

        drawer.show(entry.clone(), Some(0));
        assert!(drawer.is_enabled());
        popup.open(preview_target(Some(entry)), Some(0));
        assert!(popup.is_open());

        popup.close();
        assert!(!popup.is_open());
        assert!(drawer.is_enabled());
        assert!(!drawer.is_floating());
    });
}

#[test]
fn popup_offers_rotation_only_for_transformable_previews() {
    const TEST: &str =
        "ui::preview::popup::tests::popup_offers_rotation_only_for_transformable_previews";
    crate::test_support::gtk_test(TEST, || {
        let app = application();
        let window = gtk::ApplicationWindow::new(&app);
        let (_, popup) = popup_fixture(&window);

        popup.open(preview_target(Some(file_entry("photo.png"))), Some(0));
        assert!(popup.inner.rotate.is_visible());
        popup.close();

        popup.open(preview_target(Some(file_entry("document.pdf"))), Some(0));
        assert!(!popup.inner.rotate.is_visible());
        popup.close();
    });
}

#[test]
fn popup_navigates_items_and_updates_window_title() {
    const TEST: &str = "ui::preview::popup::tests::popup_navigates_items_and_updates_window_title";
    crate::test_support::gtk_test(TEST, || {
        let app = application();
        let window = gtk::ApplicationWindow::new(&app);
        let (drawer, popup) = popup_fixture(&window);
        let first = file_entry("photo1.png");
        let second = file_entry("photo2.png");
        let third = file_entry("photo3.png");
        let source = Rc::new(TestSource {
            entries: vec![first.clone(), second.clone(), third.clone()],
        });
        let browser = Browser::new(source);
        drawer.observe_browser(&browser);
        popup.observe_browser(&browser);

        browser.navigate(Location::local("/tmp"));
        browser.select(0, 0);
        popup.open(preview_target(Some(first)), Some(0));
        assert!(popup.is_open());
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo1.png — Quick Look")
        );

        popup.navigate_next();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo2.png".to_owned())
        );
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo2.png — Quick Look")
        );

        popup.navigate_next();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo3.png".to_owned())
        );
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo3.png — Quick Look")
        );

        popup.navigate_previous();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo2.png".to_owned())
        );
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo2.png — Quick Look")
        );

        popup.jump_first();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo1.png".to_owned())
        );
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo1.png — Quick Look")
        );

        popup.jump_last();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo3.png".to_owned())
        );
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo3.png — Quick Look")
        );

        popup.close();
        assert!(!popup.is_open());
    });
}

#[test]
fn popup_retains_open_state_with_placeholder_when_navigating_to_directory() {
    const TEST: &str = "ui::preview::popup::tests::popup_retains_open_state_with_placeholder_when_navigating_to_directory";
    crate::test_support::gtk_test(TEST, || {
        let app = application();
        let window = gtk::ApplicationWindow::new(&app);
        let (drawer, popup) = popup_fixture(&window);

        let first = file_entry("photo1.png");
        let mut dir = file_entry("subfolder");
        dir.kind = EntryKind::Directory;
        let third = file_entry("photo3.png");

        let source = Rc::new(TestSource {
            entries: vec![first.clone(), dir.clone(), third.clone()],
        });
        let browser = Browser::new(source);
        drawer.observe_browser(&browser);
        popup.observe_browser(&browser);

        browser.navigate(Location::local("/tmp"));
        browser.select(0, 1);
        popup.open(preview_target(Some(first)), Some(0));
        assert!(popup.is_open());

        popup.navigate_previous();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("subfolder".to_owned())
        );
        assert!(popup.is_open());

        popup.navigate_next();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo1.png".to_owned())
        );
        assert!(popup.is_open());

        popup.navigate_next();
        assert_eq!(
            browser.focused_entry().map(|e| e.display_name),
            Some("photo3.png".to_owned())
        );
        assert!(popup.is_open());
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo3.png — Quick Look")
        );

        popup.close();
        assert!(!popup.is_open());
    });
}

#[test]
fn popup_handles_multi_selection_cycling_and_counter_updates() {
    const TEST: &str =
        "ui::preview::popup::tests::popup_handles_multi_selection_cycling_and_counter_updates";
    crate::test_support::gtk_test(TEST, || {
        let app = application();
        let window = gtk::ApplicationWindow::new(&app);
        let (drawer, popup) = popup_fixture(&window);

        let first = file_entry("photo1.png");
        let second = file_entry("photo2.png");
        let third = file_entry("photo3.png");
        let fourth = file_entry("photo4.png");

        let source = Rc::new(TestSource {
            entries: vec![first.clone(), second.clone(), third.clone(), fourth.clone()],
        });
        let browser = Browser::new(source);
        drawer.observe_browser(&browser);
        popup.observe_browser(&browser);

        browser.navigate(Location::local("/tmp"));
        browser.select_entries_by_name_at(0, &["photo1.png".to_string(), "photo3.png".to_string()]);
        assert_eq!(browser.selected_positions(0), vec![0, 2]);

        popup.open(preview_target(Some(first)), Some(0));
        assert!(popup.is_open());
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo1.png — Quick Look")
        );
        assert_eq!(popup.inner.counter_label.text(), "1 of 2");
        assert_eq!(
            popup.inner.content_stack.visible_child_name().as_deref(),
            Some("preview")
        );

        popup.toggle_index();
        assert!(popup.index_is_visible());
        assert_eq!(popup.inner.index_summary.text(), "2 selected items");
        popup.toggle_index();
        assert!(!popup.index_is_visible());

        popup.inner.play.set_active(true);
        assert!(!popup.window().is_fullscreen());
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo3.png — Quick Look")
        );
        assert_eq!(popup.inner.counter_label.text(), "2 of 2");

        popup.navigate_next();
        assert!(!popup.inner.play.is_active());
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo1.png — Quick Look")
        );

        popup.navigate_next();
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo3.png — Quick Look")
        );

        popup.navigate_previous();
        assert_eq!(
            popup.window().title().as_deref(),
            Some("photo1.png — Quick Look")
        );

        popup.close();
        assert!(!popup.is_open());
    });
}
