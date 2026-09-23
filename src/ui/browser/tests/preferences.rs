// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, MetadataValue, SortDirection, SortKey};
use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle, LocationValidationError};

struct PhotoOrderSource;

impl FileSource for PhotoOrderSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: [("z.jpg", 10), ("b.jpg", 30), ("a.jpg", 20)]
                .into_iter()
                .map(|(name, size)| FileEntry {
                    location: request
                        .location
                        .child(std::ffi::OsStr::new(name))
                        .expect("fixture photo location"),
                    native_name: name.into(),
                    display_name: name.into(),
                    thumbnail_path: None,
                    kind: EntryKind::File,
                    size: MetadataValue::Known(size),
                    modified_unix_seconds: MetadataValue::Known(1),
                    mode: MetadataValue::Known(0o100644),
                    recent_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                })
                .collect(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: Some(false),
            can_delete: Some(true),
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn camera_device_order_overrides_saved_sort_without_changing_other_windows_or_rebuilt_views() {
    crate::test_support::gtk_test(
        "ui::browser::tests::preferences::camera_device_order_overrides_saved_sort_without_changing_other_windows_or_rebuilt_views",
        || {
            use crate::ui::preferences::PreferenceManager;
            PreferenceManager::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let saved = manager.sort_preferences();
            assert_eq!(saved.sort_key, SortKey::Size);
            let wait = |done: &dyn Fn() -> bool| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !done() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "photo ordering did not settle"
                    );
                    glib::MainContext::default().iteration(false);
                }
            };
            let views = [
                BrowserView::new(Rc::new(PhotoOrderSource), PeekBehavior::default()),
                BrowserView::new(Rc::new(PhotoOrderSource), PeekBehavior::default()),
            ];
            let windows: Vec<_> = views
                .iter()
                .map(|view| {
                    let window = gtk::Window::builder().child(&view.widget()).build();
                    window.present();
                    window
                })
                .collect();
            let camera = views[0].browser();
            let folder = views[1].browser();
            camera.navigate(Location::uri("gphoto2://camera/"));
            folder.navigate(Location::local("/fixture"));
            wait(&|| {
                [camera.clone(), folder.clone()].iter().all(|browser| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
                })
            });
            assert_eq!(
                camera
                    .entry_at(0, 0)
                    .expect("first discovered photo")
                    .display_name,
                "z.jpg"
            );
            assert_eq!(
                folder.entry_at(0, 0).expect("largest file").display_name,
                "b.jpg"
            );
            assert_eq!(manager.sort_preferences(), saved);
            for mode in [BrowserMode::Icons, BrowserMode::List, BrowserMode::Columns] {
                views[0].set_view_mode(mode);
                assert_eq!(
                    camera
                        .entry_at(0, 0)
                        .expect("first discovered photo after rebuild")
                        .display_name,
                    "z.jpg"
                );
                assert_eq!(
                    camera
                        .column_preferences(0)
                        .expect("Photos preferences")
                        .sort_key,
                    SortKey::DeviceOrder
                );
            }
            let direction = views[0].state.columns.borrow()[0]
                .sort_direction_button
                .clone();
            assert!(!direction.is_sensitive());
            camera.set_sort(0, SortKey::Name, SortDirection::Ascending);
            wait(&|| {
                direction.is_sensitive()
                    && camera
                        .entry_at(0, 0)
                        .is_some_and(|entry| entry.display_name == "a.jpg")
            });
            assert_eq!(manager.sort_preferences().sort_key, SortKey::Name);
            assert_eq!(
                folder
                    .column_preferences(0)
                    .expect("existing folder preferences")
                    .sort_key,
                SortKey::Size
            );
            assert_eq!(
                folder
                    .entry_at(0, 0)
                    .expect("largest file still first")
                    .display_name,
                "b.jpg"
            );
            direction.emit_clicked();
            wait(&|| {
                camera
                    .entry_at(0, 0)
                    .is_some_and(|entry| entry.display_name == "z.jpg")
            });
            camera.set_sort_key(0, SortKey::DeviceOrder);
            wait(&|| {
                !direction.is_sensitive()
                    && camera
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
            });
            assert_eq!(
                camera
                    .entry_at(0, 1)
                    .expect("second discovered photo")
                    .display_name,
                "b.jpg"
            );
            assert_eq!(manager.sort_preferences().sort_key, SortKey::Name);
            folder.navigate(Location::local("/another-folder"));
            wait(&|| {
                folder
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            assert_eq!(
                folder
                    .entry_at(0, 0)
                    .expect("descending name sort")
                    .display_name,
                "z.jpg"
            );
            assert_eq!(
                folder
                    .column_preferences(0)
                    .expect("new folder preferences")
                    .sort_key,
                SortKey::Name
            );
            let defaults = manager.sort_preferences();
            manager.set_sort_preferences(camera.column_preferences(0).expect("Photos preferences"));
            assert_eq!(manager.sort_preferences(), defaults);
            for window in windows {
                window.close();
            }
        },
    );
}

impl BrowserView {
    pub(in crate::ui) fn assert_saved_preferences(
        &self,
        manager: &crate::ui::preferences::PreferenceManager,
    ) {
        assert_eq!(self.state.peek_enabled.get(), manager.folder_peeking());
        assert_eq!(
            crate::sandbox::browser::worker_limit(),
            manager.thumbnail_workers()
        );
        assert_eq!(
            self.single_click_previews_enabled(),
            manager.single_click_previews()
        );
        assert_eq!(
            self.columns_mirror_selection_enabled(),
            manager.columns_mirror_selection()
        );
        assert_eq!(
            self.state.columns_click_activation.get(),
            manager.click_activation(BrowserMode::Columns)
        );
        assert_eq!(
            self.state.auto_refresh.borrow().is_some(),
            manager.auto_refresh_interval() != 0
        );
        assert_eq!(self.browser().preferences(), manager.sort_preferences());
        self.state
            .mode_views
            .borrow()
            .assert_saved_preferences(manager);
    }

    pub(in crate::ui) fn assert_peek_scheduling(&self, enabled: bool) {
        self.state.input_ownership.borrow_mut().last_navigation =
            crate::ui::input_ownership::NavigationInput::Pointer;
        self.state.schedule_peek(
            0,
            Location::local("/fixture/child"),
            gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
        );
        assert_eq!(self.state.pending_peek.borrow().is_some(), enabled);
        cancel_source(&self.state.pending_peek);
    }
}
