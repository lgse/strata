// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use super::super::{
    RecentAvailability, browser_for_window, home_directory, save_pinned_places,
    should_show_standard_place, sidebar_button, standard_place,
};
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
            PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
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
            let preferences = PreferenceManager::shared();
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
            let sidebar = build_sidebar(view, PreferenceManager::shared(), true);
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
            let recent = Location::uri("recent:///");
            let recent_row = sidebar_button(crate::assets::icons::CLOCK, "Recent");
            sidebar
                .state
                .bind_place_row(&recent_row, recent.clone(), PlaceNavigation::Direct);
            sidebar.state.widget.append(&recent_row);
            recent_row.emit_clicked();
            assert_eq!(sidebar.state.browser.active_location(), Some(recent));
            assert!(recent_row.has_css_class("active"));
            assert!(!validated_row.has_css_class("active"));
            let recent_controllers = recent_row.observe_controllers();
            assert_eq!(
                (0..recent_controllers.n_items())
                    .filter_map(|index| {
                        recent_controllers
                            .item(index)?
                            .downcast::<gtk::DropTarget>()
                            .ok()
                    })
                    .count(),
                0
            );
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
            let sidebar = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            assert_eq!(sidebar.handlers.borrow().len(), 9);
            let callback = rebuild_on_change::<()>(&sidebar.state);
            let monitor = sidebar.state.volume_monitor.clone();
            let initial = sidebar
                .state
                .widget
                .first_child()
                .expect("initial Home row");
            callback(&monitor, &());
            drain_main_context();
            let before = sidebar.state.widget.first_child().expect("Home row");
            assert_ne!(initial, before);
            sidebar
                .state
                .mount_monitor
                .emit_by_name::<()>("mounts-changed", &[]);
            drain_main_context();
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

#[test]
fn rebuild_preserves_scrolled_offset() {
    gtk_test(
        "ui::window::sidebar::tests::rebuild_preserves_scrolled_offset",
        || {
            let pins = (0..30)
                .map(|index| {
                    (
                        Location::local(home_directory().join(format!("pin-{index}"))),
                        format!("Pin {index}"),
                    )
                })
                .collect::<Vec<_>>();
            save_pinned_places(&pins).expect("seed bookmarks");
            let sidebar = build_sidebar(browser_for_window(), PreferenceManager::shared(), false);
            let window = gtk::Window::builder()
                .child(&sidebar.widget)
                .default_width(240)
                .default_height(140)
                .build();
            window.present();
            let scroller = sidebar_scroller(&sidebar);
            wait_until(|| {
                let adjustment = scroller.vadjustment();
                adjustment.upper() > adjustment.page_size() + 80.0
            });
            let requested = 80.0;
            scroller.vadjustment().set_value(requested);
            settle_mapped(&window);
            assert!(
                (scroller.vadjustment().value() - requested).abs() < 1.0,
                "seeded scroll offset"
            );
            let callback = rebuild_on_change::<()>(&sidebar.state);
            let monitor = sidebar.state.volume_monitor.clone();
            callback(&monitor, &());
            callback(&monitor, &());
            settle_mapped(&window);
            assert!(
                (scroller.vadjustment().value() - requested).abs() < 1.0,
                "kept scroll offset after device rebuild, got {}",
                scroller.vadjustment().value()
            );
            sidebar.disconnect();
            sidebar.state.browser.clear_observer();
            window.destroy();
        },
    );
}

fn sidebar_scroller(sidebar: &SidebarView) -> gtk::ScrolledWindow {
    sidebar
        .widget
        .first_child()
        .and_downcast()
        .expect("sidebar scroller")
}

fn drain_main_context() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle_mapped(window: &gtk::Window) {
    let frames = Rc::new(Cell::new(0));
    let observed = frames.clone();
    window.add_tick_callback(move |_, _| {
        observed.set(observed.get() + 1);
        if observed.get() >= 3 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_until(|| frames.get() >= 3);
    drain_main_context();
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "sidebar layout timed out");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn has_location(sidebar: &SidebarView, location: &Location) -> bool {
    sidebar
        .state
        .place_rows
        .borrow()
        .iter()
        .any(|(candidate, _)| candidate == location)
}

fn standard_location(id: &str) -> Option<Location> {
    let (_, _, directory) = standard_place(id)?;
    let path = glib::user_special_dir(directory)?;
    if !should_show_standard_place(id, &path, &home_directory()) {
        return None;
    }
    Some(Location::local(path))
}

fn context_action(widget: &gtk::Widget, title: &str) -> Option<gtk::Button> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.text() == title
    {
        return widget.ancestor(gtk::Button::static_type()).and_downcast();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = context_action(&widget, title) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn recent_place_appears_by_default_when_runtime_is_available() {
    gtk_test(
        "ui::window::sidebar::tests::recent_place_appears_by_default_when_runtime_is_available",
        || {
            let sidebar = build_sidebar(browser_for_window(), PreferenceManager::shared(), false);
            sidebar.state.recent_availability.set(RecentAvailability {
                platform_tracking_enabled: true,
                runtime_backend_supported: true,
            });
            sidebar.state.rebuild();
            assert!(has_location(&sidebar, &Location::uri("recent:///")));
            sidebar.disconnect();
            sidebar.state.browser.clear_observer();
        },
    );
}

#[test]
fn sidebar_visibility_prefs_hide_and_restore_default_places_across_windows() {
    gtk_test(
        "ui::window::sidebar::tests::sidebar_visibility_prefs_hide_and_restore_default_places_across_windows",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            assert_eq!(
                manager.sidebar_places_visibility(),
                [
                    false, false, false, false, false, false, false, false, false
                ]
            );
            let sidebars = [
                build_sidebar(browser_for_window(), manager.clone(), false),
                build_sidebar(browser_for_window(), manager.clone(), false),
            ];
            let home = Location::local(home_directory());
            let trash = Location::uri("trash:///");
            let network = Location::uri("network:///");
            let recent = Location::uri("recent:///");
            for sidebar in &sidebars {
                sidebar.state.recent_availability.set(RecentAvailability {
                    platform_tracking_enabled: true,
                    runtime_backend_supported: true,
                });
                sidebar.state.rebuild();
            }
            let standard_ids = ["desktop", "documents", "downloads", "pictures", "videos"];
            let standards: Vec<(&str, Location)> = standard_ids
                .into_iter()
                .filter_map(|id| standard_location(id).map(|location| (id, location)))
                .collect();
            for sidebar in &sidebars {
                assert!(!has_location(sidebar, &home));
                assert!(!has_location(sidebar, &trash));
                assert!(!has_location(sidebar, &network));
                assert!(!has_location(sidebar, &recent));
                for (_, location) in &standards {
                    assert!(!has_location(sidebar, location));
                }
            }
            manager.set_sidebar_show_home(true);
            manager.set_sidebar_show_trash(true);
            manager.set_sidebar_show_network(true);
            manager.set_sidebar_show_recent(true);
            manager.set_sidebar_show_desktop(true);
            manager.set_sidebar_show_documents(true);
            manager.set_sidebar_show_downloads(true);
            manager.set_sidebar_show_pictures(true);
            manager.set_sidebar_show_videos(true);
            for sidebar in &sidebars {
                assert!(has_location(sidebar, &home));
                assert!(has_location(sidebar, &trash));
                assert!(has_location(sidebar, &network));
                assert!(has_location(sidebar, &recent));
                for (_, location) in &standards {
                    assert!(has_location(sidebar, location));
                }
            }
            manager.set_sidebar_show_recent(false);
            for sidebar in &sidebars {
                assert!(!has_location(sidebar, &recent));
                assert!(has_location(sidebar, &home));
                assert!(has_location(sidebar, &trash));
                assert!(has_location(sidebar, &network));
                for (_, location) in &standards {
                    assert!(has_location(sidebar, location));
                }
            }
            manager.set_sidebar_show_recent(true);
            for sidebar in &sidebars {
                assert!(has_location(sidebar, &recent));
            }
            let chooser = build_sidebar(browser_for_window(), manager.clone(), true);
            chooser.state.recent_availability.set(RecentAvailability {
                platform_tracking_enabled: true,
                runtime_backend_supported: true,
            });
            chooser.state.rebuild();
            assert!(has_location(&chooser, &home));
            assert!(!has_location(&chooser, &trash));
            assert!(!has_location(&chooser, &network));
            assert!(has_location(&chooser, &recent));
            row(&chooser, &recent).emit_clicked();
            assert_eq!(
                chooser.state.browser.active_location(),
                Some(recent.clone())
            );
            manager.set_sidebar_show_recent(false);
            assert!(!has_location(&chooser, &recent));
            manager.set_sidebar_show_recent(true);
            assert!(has_location(&chooser, &recent));
            for (index, location) in [&home, &trash, &network]
                .into_iter()
                .chain(standards.iter().map(|(_, location)| location))
                .enumerate()
            {
                let place = row(&sidebars[index % 2], location);
                assert!(context_action(place.upcast_ref(), "Properties").is_some());
                context_action(place.upcast_ref(), "Unpin")
                    .expect("default place Unpin action")
                    .emit_clicked();
                for sidebar in &sidebars {
                    assert!(!has_location(sidebar, location));
                }
            }
            manager.set_sidebar_show_recent(false);
            for sidebar in &sidebars {
                assert!(!has_location(sidebar, &recent));
            }
            assert!(!manager.sidebar_show_home());
            assert!(!manager.sidebar_show_trash());
            assert!(!manager.sidebar_show_network());
            assert!(!manager.sidebar_show_recent());
            for sidebar in sidebars.iter().chain(std::iter::once(&chooser)) {
                assert!(!has_location(sidebar, &home));
                for (_, location) in &standards {
                    assert!(!has_location(sidebar, location));
                }
            }
            for sidebar in &sidebars {
                assert!(!has_location(sidebar, &trash));
                assert!(!has_location(sidebar, &network));
                assert!(!has_location(sidebar, &recent));
            }
            for sidebar in sidebars {
                sidebar.disconnect();
                sidebar.state.browser.clear_observer();
            }
            chooser.disconnect();
            chooser.state.browser.clear_observer();
        },
    );
}

#[test]
fn schedule_after_first_paint_rebuilds_sidebar_places() {
    gtk_test(
        "ui::window::sidebar::tests::schedule_after_first_paint_rebuilds_sidebar_places",
        || {
            let sidebar = build_sidebar(browser_for_window(), PreferenceManager::shared(), true);
            let window = gtk::Window::builder()
                .child(&sidebar.widget)
                .default_width(240)
                .default_height(140)
                .build();
            let initial = sidebar
                .state
                .widget
                .first_child()
                .expect("initial Home row");
            sidebar.schedule_after_first_paint(&window);
            window.present();
            settle_mapped(&window);
            let rebuilt = sidebar
                .state
                .widget
                .first_child()
                .expect("rebuilt Home row");
            assert_ne!(initial, rebuilt);
            sidebar.disconnect();
            sidebar.state.browser.clear_observer();
            window.destroy();
        },
    );
}
