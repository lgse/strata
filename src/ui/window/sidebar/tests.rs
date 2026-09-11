// SPDX-License-Identifier: MIT

use super::super::{browser_for_window, home_directory, save_pinned_places, sidebar_button};
use super::*;
use crate::{
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::gtk_test,
    ui::browser::PeekBehavior,
};

fn row(sidebar: &SidebarView, location: &Location) -> gtk::Button {
    sidebar
        .state
        .place_rows
        .borrow()
        .iter()
        .find(|(candidate, _)| candidate == location)
        .map(|(_, row)| row.clone())
        .expect("sidebar location row")
}

fn pinned_locations(sidebar: &SidebarView) -> Vec<Location> {
    sidebar
        .state
        .pinned_places
        .borrow()
        .iter()
        .map(|(location, _)| location.clone())
        .collect()
}

#[test]
fn saved_order_rebuilds_both_sidebars_without_losing_active_places() {
    gtk_test(
        "ui::window::sidebar::tests::saved_order_rebuilds_both_sidebars_without_losing_active_places",
        || {
            ThemeManager::seed_saved_preferences_for_test();
            let preferences = ThemeManager::shared();
            let location = Location::local(home_directory().join("fixture-pin"));
            save_pinned_places(&[(location.clone(), "Pinned fixture".into())])
                .expect("seed bookmarks");
            let sidebars = [
                build_sidebar(browser_for_window(), preferences.clone(), true),
                build_sidebar(browser_for_window(), preferences.clone(), true),
            ];
            for sidebar in &sidebars {
                assert_eq!(
                    *sidebar.state.place_order.borrow(),
                    resolve_place_order(&preferences.sidebar_order())
                );
                sidebar.state.browser.navigate(location.clone());
                assert!(row(sidebar, &location).has_css_class("active"));
            }
            for order in [
                vec!["downloads", "desktop", "pictures", "documents", "videos"],
                vec!["videos", "pictures", "documents", "desktop", "downloads"],
            ] {
                let before = sidebars.each_ref().map(|sidebar| row(sidebar, &location));
                preferences.set_sidebar_order(order.iter().map(|id| (*id).to_owned()).collect());
                for (sidebar, before) in sidebars.iter().zip(before) {
                    assert_eq!(*sidebar.state.place_order.borrow(), order);
                    let after = row(sidebar, &location);
                    assert_ne!(before, after);
                    assert!(after.has_css_class("active"));
                    assert!(!sidebar.update_area.get_visible());
                }
            }
            for sidebar in sidebars {
                sidebar.disconnect();
                sidebar.state.browser.clear_observer();
            }
        },
    );
}

#[test]
fn pinned_row_reordering_preserves_storage_and_chooser_filtering() {
    gtk_test(
        "ui::window::sidebar::tests::pinned_row_reordering_preserves_storage_and_chooser_filtering",
        || {
            let preferences = ThemeManager::shared();
            let sidebar = build_sidebar(browser_for_window(), preferences.clone(), false);
            let first = Location::local(home_directory().join("first-pin"));
            let second = Location::local(home_directory().join("second-pin"));
            let remote =
                crate::adapters::location_for_file(&gio::File::for_uri("smb://example.test/share"))
                    .expect("remote bookmark location");
            sidebar.state.pin_location(first.clone(), "First".into());
            sidebar.state.pin_location(second.clone(), "Second".into());
            sidebar.state.pin_location(remote.clone(), "Remote".into());
            sidebar
                .state
                .pin_location(first.clone(), "Duplicate".into());
            assert_eq!(
                pinned_locations(&sidebar),
                [first.clone(), second.clone(), remote.clone()]
            );
            sidebar.state.browser.navigate(first.clone());
            sidebar.state.reorder_pinned_place(0, 1, true);
            assert_eq!(
                pinned_locations(&sidebar),
                [second.clone(), first.clone(), remote.clone()]
            );
            assert!(row(&sidebar, &first).has_css_class("active"));
            assert!(row(&sidebar, &first).has_css_class("reorderable"));
            assert!(row(&sidebar, &first).has_css_class("file-drop-zone"));
            let chooser = build_sidebar(browser_for_window(), preferences, true);
            assert_eq!(pinned_locations(&chooser), pinned_locations(&sidebar));
            assert!(!row(&chooser, &first).has_css_class("reorderable"));
            assert!(
                chooser
                    .state
                    .place_rows
                    .borrow()
                    .iter()
                    .all(|(location, _)| location.native_path().is_some())
            );
            sidebar.state.unpin_location(&first);
            assert_eq!(pinned_locations(&sidebar), [second, remote]);
            assert_eq!(
                load_pinned_places().expect("saved bookmarks"),
                *sidebar.state.pinned_places.borrow()
            );
            for sidebar in [sidebar, chooser] {
                sidebar.disconnect();
                sidebar.state.browser.clear_observer();
            }
        },
    );
}

#[derive(Default)]
struct NavigationSource {
    validations: Cell<usize>,
    enumerations: Cell<usize>,
}

impl FileSource for NavigationSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        self.validations.set(self.validations.get() + 1);
        Ok(())
    }

    fn enumerate(&self, _: DirectoryRequest, _: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.enumerations.set(self.enumerations.get() + 1);
        LoadHandle::new(|| {})
    }
}

#[test]
fn shared_place_bindings_keep_navigation_and_drop_policies_distinct() {
    gtk_test(
        "ui::window::sidebar::tests::shared_place_bindings_keep_navigation_and_drop_policies_distinct",
        || {
            let source = Rc::new(NavigationSource::default());
            let view = BrowserView::new(source.clone(), PeekBehavior::default());
            let sidebar = build_sidebar(view, ThemeManager::shared(), true);
            let direct = Location::uri("fixture:///direct");
            let validated = Location::uri("fixture:///validated");
            let direct_row = sidebar_button(crate::assets::icons::FOLDER, "Direct");
            let validated_row = sidebar_button(crate::assets::icons::FOLDER, "Validated");
            sidebar
                .state
                .bind_place_row(&direct_row, direct.clone(), PlaceNavigation::Direct);
            sidebar.state.bind_place_row(
                &validated_row,
                validated.clone(),
                PlaceNavigation::Validate,
            );
            sidebar.state.widget.append(&direct_row);
            sidebar.state.widget.append(&validated_row);
            direct_row.emit_clicked();
            assert_eq!(source.validations.get(), 0);
            assert_eq!(sidebar.state.browser.active_location(), Some(direct));
            assert!(direct_row.has_css_class("active"));
            validated_row.emit_clicked();
            assert_eq!(source.validations.get(), 1);
            assert_eq!(source.enumerations.get(), 2);
            assert_eq!(sidebar.state.browser.active_location(), Some(validated));
            assert!(validated_row.has_css_class("active"));
            assert!(!direct_row.has_css_class("active"));
            let trash = sidebar_button(crate::assets::icons::TRASH, "Trash");
            sidebar.state.bind_place_row(
                &trash,
                Location::uri("trash:///"),
                PlaceNavigation::Direct,
            );
            assert!(trash.has_css_class("file-drop-zone"));
            let controllers = trash.observe_controllers();
            let targets: Vec<_> = (0..controllers.n_items())
                .filter_map(|index| controllers.item(index)?.downcast::<gtk::DropTarget>().ok())
                .collect();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].actions(), gtk::gdk::DragAction::MOVE);
            assert!(targets[0].is_preload());
            sidebar.disconnect();
            sidebar.state.browser.clear_observer();
        },
    );
}

#[test]
fn device_subscriptions_rebuild_until_disconnected_and_capture_state_weakly() {
    gtk_test(
        "ui::window::sidebar::tests::device_subscriptions_rebuild_until_disconnected_and_capture_state_weakly",
        || {
            let sidebar = build_sidebar(browser_for_window(), ThemeManager::shared(), true);
            assert_eq!(sidebar.handlers.borrow().len(), 6);
            let callback = rebuild_on_change::<()>(&sidebar.state);
            let monitor = sidebar.state.volume_monitor.clone();
            let initial = sidebar
                .state
                .widget
                .first_child()
                .expect("initial Home row");
            callback(&monitor, &());
            let before = sidebar.state.widget.first_child().expect("Home row");
            assert_ne!(initial, before);
            sidebar
                .state
                .mount_monitor
                .emit_by_name::<()>("mounts-changed", &[]);
            let rebuilt = sidebar
                .state
                .widget
                .first_child()
                .expect("rebuilt Home row");
            assert_ne!(before, rebuilt);
            sidebar.disconnect();
            sidebar.disconnect();
            assert!(sidebar.handlers.borrow().is_empty());
            assert!(sidebar.mount_handler.borrow().is_none());
            sidebar
                .state
                .mount_monitor
                .emit_by_name::<()>("mounts-changed", &[]);
            assert_eq!(sidebar.state.widget.first_child(), Some(rebuilt));
            let weak = Rc::downgrade(&sidebar.state);
            sidebar.state.browser.clear_observer();
            drop(sidebar);
            assert!(weak.upgrade().is_none());
            callback(&monitor, &());
        },
    );
}
