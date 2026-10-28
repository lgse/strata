// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::prelude::*;

use super::super::{
    browser_for_window, build_sidebar, move_pinned_place, pin_gap_index, pin_locations_at,
    sidebar_reorder_matches,
};
use crate::model::Location;
use crate::test_support::gtk_test_with_env;
use crate::ui::preferences::PreferenceManager;

fn wait_until(condition: impl Fn() -> bool, waiting_for: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {waiting_for}"
        );
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn saved_pins() -> Vec<(Location, String)> {
    crate::storage::writer::settle();
    let contents =
        std::fs::read(crate::adapters::bookmarks::pinned_places_path()).unwrap_or_default();
    crate::adapters::bookmarks::parse_pinned_places(&contents)
}

fn save_pins(places: &[(Location, String)]) {
    let path = crate::adapters::bookmarks::pinned_places_path();
    std::fs::create_dir_all(path.parent().expect("config folder")).expect("config folder");
    std::fs::write(
        &path,
        crate::adapters::bookmarks::serialize_pinned_places(places),
    )
    .expect("saved pins");
}

fn shows_label(widget: &gtk::Widget, text: &str) -> bool {
    if !widget.is_visible() {
        return false;
    }
    if widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.label() == text)
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if shows_label(&current, text) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

#[test]
fn a_rejected_place_click_returns_the_highlight_to_the_open_location() {
    // Without a GVfs backend, network:// is rejected rather than opened.
    gtk_test_with_env(
        "ui::window::tests::sidebar::a_rejected_place_click_returns_the_highlight_to_the_open_location",
        [("GIO_USE_VFS", "local")],
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            preferences.set_sidebar_show_home(true);
            preferences.set_sidebar_show_network(true);
            let view = browser_for_window();
            let sidebar = build_sidebar(view.clone(), preferences, false);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            content.append(&sidebar.widget);
            content.append(&view.widget());
            let window = gtk::Window::builder()
                .child(&content)
                .default_width(1000)
                .default_height(600)
                .build();
            window.present();
            let row = |location: &Location| {
                sidebar
                    .state
                    .place_rows
                    .borrow()
                    .iter()
                    .find(|(candidate, _)| candidate == location)
                    .map(|(_, row)| row.clone())
                    .expect("sidebar place row")
            };
            let home = Location::local(gtk::glib::home_dir());
            let network = Location::uri("network:///");

            row(&home).emit_clicked();
            wait_until(
                || view.browser().active_location() == Some(home.clone()),
                "Home to open",
            );
            row(&network).emit_clicked();
            wait_until(
                || shows_label(&view.widget(), "Unable to open location"),
                "the Network rejection",
            );

            assert_eq!(view.browser().active_location(), Some(home.clone()));
            assert!(
                row(&home).has_css_class("active"),
                "Home stays open, so its row must keep the highlight"
            );
            assert!(
                !row(&network).has_css_class("active"),
                "the rejected Network row must not stay highlighted"
            );
            view.browser().clear_observer();
            sidebar.disconnect();
            window.destroy();
        },
    );
}

#[test]
fn navigation_and_history_keep_only_the_current_sidebar_place_active() {
    gtk_test_with_env(
        "ui::window::tests::sidebar::navigation_and_history_keep_only_the_current_sidebar_place_active",
        [("GIO_USE_VFS", "local")],
        || {
            let directory = tempfile::tempdir().expect("places fixture");
            let a = Location::local(directory.path().join("a"));
            let b = Location::local(directory.path().join("b"));
            let child = Location::local(directory.path().join("a/child"));
            std::fs::create_dir_all(child.native_path().expect("native child folder"))
                .expect("child folder");
            std::fs::create_dir(b.native_path().expect("native second folder"))
                .expect("second folder");
            save_pins(&[(a.clone(), "A".into()), (b.clone(), "B".into())]);
            let view = browser_for_window();
            let sidebar = build_sidebar(view.clone(), PreferenceManager::shared(), false);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            content.append(&sidebar.widget);
            content.append(&view.widget());
            let window = gtk::Window::builder().child(&content).build();
            window.present();
            let click = |location: &Location| {
                let row = sidebar
                    .state
                    .place_rows
                    .borrow()
                    .iter()
                    .find(|(candidate, _)| candidate == location)
                    .expect("place row")
                    .1
                    .clone();
                row.emit_clicked();
            };
            let expect = |location: &Location| {
                wait_until(
                    || view.browser().active_location().as_ref() == Some(location),
                    "navigation to commit",
                );
                for (candidate, row) in sidebar.state.place_rows.borrow().iter() {
                    assert_eq!(
                        row.has_css_class("active"),
                        candidate == location,
                        "current={location:?}, row={candidate:?}"
                    );
                }
            };
            click(&a);
            expect(&a);
            view.browser().navigate_location(child.clone(), true);
            expect(&child);
            view.browser().parent();
            expect(&a);
            click(&b);
            expect(&b);
            view.browser().back();
            expect(&a);
            view.browser().navigate_location(b.clone(), true);
            expect(&b);
            view.browser().clear_observer();
            sidebar.disconnect();
            window.destroy();
        },
    );
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        widgets.extend(descendants(&current));
        child = current.next_sibling();
    }
    widgets
}

#[track_caller]
fn find<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>, matches: impl Fn(&T) -> bool) -> T {
    descendants(root.as_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<T>().ok())
        .find(|widget| matches(widget))
        .expect("widget")
}

/// Unlike [`shows_label`], also matches inside a closed popover.
fn find_label(root: &gtk::Widget, text: &str) -> bool {
    descendants(root)
        .iter()
        .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
        .any(|label| label.label() == text)
}

fn modal_layers(overlay: &gtk::Overlay) -> usize {
    descendants(overlay.upcast_ref())
        .iter()
        .filter(|widget| {
            widget.has_css_class("app-modal-layer") && !widget.has_css_class("dismissing")
        })
        .count()
}

#[test]
fn connect_to_server_stays_open_until_connecting_ends_and_pins_only_on_success() {
    // Without a GVfs backend an SMB share fails quickly and never reaches the network.
    gtk_test_with_env(
        "ui::window::tests::sidebar::connect_to_server_stays_open_until_connecting_ends_and_pins_only_on_success",
        [("GIO_USE_VFS", "local")],
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            let preferences = PreferenceManager::shared();
            preferences.set_sidebar_show_network(true);
            let view = browser_for_window();
            let sidebar = build_sidebar(view.clone(), preferences, false);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            content.append(&sidebar.widget);
            content.append(&view.widget());
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&content));
            let window = gtk::Window::builder()
                .child(&overlay)
                .default_width(1000)
                .default_height(600)
                .build();
            window.present();
            let network = Location::uri("network:///");
            let network_row = sidebar
                .state
                .place_rows
                .borrow()
                .iter()
                .find(|(candidate, _)| *candidate == network)
                .map(|(_, row)| row.clone())
                .expect("Network row");
            let pins_before = saved_pins();

            find::<gtk::Button>(&network_row, |button| {
                button
                    .child()
                    .is_some_and(|child| find_label(&child, "Connect to server…"))
            })
            .emit_clicked();
            assert_eq!(modal_layers(&overlay), 1, "the dialog opens");
            let address = find::<gtk::Entry>(&overlay, |entry| entry.has_css_class("form-control"));
            let pin = find::<gtk::CheckButton>(&overlay, |_| true);
            let connect = find::<gtk::Button>(&overlay, |button| {
                button.has_css_class("action-dialog-confirm")
            });
            address.set_text("smb://unreachable.invalid/share");
            pin.set_active(true);
            let clicked = Instant::now();
            connect.emit_clicked();

            assert_eq!(
                modal_layers(&overlay),
                1,
                "the dialog stays while connecting"
            );
            assert!(!connect.is_sensitive() && !address.is_sensitive() && !pin.is_sensitive());
            let spinner = || {
                find::<gtk::Spinner>(&overlay, |spinner| {
                    spinner.has_css_class("action-dialog-loading")
                })
            };
            assert!(spinner().is_visible() && spinner().is_spinning());

            wait_until(|| connect.is_sensitive(), "connecting to fail");
            assert!(
                clicked.elapsed() >= super::super::network_location::MIN_CONNECTING,
                "an instant failure still reads as an attempt"
            );
            assert_eq!(
                modal_layers(&overlay),
                1,
                "the failure is shown in the dialog, not a second one"
            );
            assert!(address.is_sensitive() && pin.is_sensitive());
            assert!(!spinner().is_visible());
            assert!(address.has_css_class("error"));
            let error =
                find::<gtk::Label>(&overlay, |label| label.has_css_class("form-field-error"));
            assert!(error.is_visible() && !error.text().is_empty());
            assert_eq!(saved_pins(), pins_before);
            assert_ne!(
                view.browser().active_location(),
                Some(Location::uri("smb://unreachable.invalid/share"))
            );

            let folder = tempfile::tempdir().expect("folder to connect to");
            let opened = Location::local(folder.path());
            address.set_text(&folder.path().to_string_lossy());
            connect.emit_clicked();
            wait_until(
                || modal_layers(&overlay) == 0,
                "the dialog to close once the location opens",
            );
            assert_eq!(view.browser().active_location(), Some(opened.clone()));
            assert!(
                saved_pins().iter().any(|(location, _)| *location == opened),
                "the retried location is pinned"
            );
            view.browser().clear_observer();
            sidebar.disconnect();
            window.destroy();
        },
    );
}

#[test]
fn an_empty_pinned_section_shows_only_while_files_are_dragged_over_the_sidebar() {
    crate::test_support::gtk_test(
        "ui::window::tests::sidebar::an_empty_pinned_section_shows_only_while_files_are_dragged_over_the_sidebar",
        || {
            PreferenceManager::seed_saved_preferences_for_test();
            save_pins(&[]);
            let view = browser_for_window();
            let sidebar = build_sidebar(view.clone(), PreferenceManager::shared(), false);
            let window = gtk::Window::builder().child(&sidebar.widget).build();
            window.present();
            let pinned_shown = || shows_label(&sidebar.widget, "PINNED");

            assert!(!pinned_shown(), "nothing pinned, so no Pinned section");
            sidebar.state.set_file_drag_over(true);
            assert!(pinned_shown(), "a file drag reveals the pin drop target");
            sidebar.state.rebuild();
            assert!(
                pinned_shown(),
                "a rebuild during the drag keeps it revealed"
            );
            sidebar.state.set_file_drag_over(false);
            assert!(!pinned_shown(), "leaving the sidebar hides it again");

            sidebar
                .state
                .pin_location(Location::local("/pinned/folder"), "Folder".to_owned());
            wait_until(|| shows_label(&sidebar.widget, "Folder"), "the new pin");
            assert!(pinned_shown(), "a section with pins always shows");
            view.browser().clear_observer();
            sidebar.disconnect();
            window.destroy();
        },
    );
}

#[test]
fn sidebar_reorders_only_distinct_rows_in_the_same_section() {
    for (source, target, accepts) in [
        ("trash", "home", true),
        ("home", "trash", true),
        ("pinned:0", "pinned:1", true),
        ("pinned:1", "pinned:0", true),
        ("home", "home", false),
        ("pinned:0", "pinned:0", false),
        ("home", "pinned:0", false),
        ("pinned:0", "home", false),
        ("not-a-place", "home", false),
        ("pinned:invalid", "pinned:0", false),
    ] {
        assert_eq!(
            sidebar_reorder_matches(source, target),
            accepts,
            "{source} -> {target}"
        );
    }
}

fn pin(path: &str, name: &str) -> (Location, String) {
    (Location::local(path), name.to_owned())
}

#[test]
fn pin_locations_at_inserts_at_specified_indices_with_their_display_names() {
    let mut places = vec![pin("/pinned/a", "A"), pin("/pinned/b", "B")];
    for (folder, index) in [
        (pin("/pinned/first", "First"), 0),
        (pin("/pinned/middle", "Middle"), 2),
        (pin("/pinned/last", "Last"), 99),
    ] {
        assert!(pin_locations_at(&mut places, &[folder], index));
    }
    assert_eq!(
        places,
        vec![
            pin("/pinned/first", "First"),
            pin("/pinned/a", "A"),
            pin("/pinned/middle", "Middle"),
            pin("/pinned/b", "B"),
            pin("/pinned/last", "Last"),
        ]
    );
}

#[test]
fn pin_locations_at_skips_existing_and_standard_places_without_reordering() {
    let original = vec![
        pin("/pinned/a", "A"),
        pin("/pinned/b", "B"),
        pin("/pinned/c", "C"),
    ];
    let home = (Location::local(gtk::glib::home_dir()), "Home".to_owned());
    for sources in [vec![pin("/pinned/b", "Renamed")], vec![home]] {
        let mut places = original.clone();
        assert!(!pin_locations_at(&mut places, &sources, 0));
        assert_eq!(places, original);
    }
}

#[test]
fn pin_locations_at_multi_folder_drag_preserves_selection_order_and_skips_duplicates() {
    let mut places = vec![pin("/pinned/a", "A"), pin("/pinned/b", "B")];

    let changed = pin_locations_at(
        &mut places,
        &[
            pin("/pinned/new_1", "New 1"),
            pin("/pinned/a", "A"),
            pin("/pinned/new_1", "New 1"),
            pin("/pinned/new_2", "New 2"),
        ],
        1,
    );
    assert!(changed);
    assert_eq!(
        places,
        vec![
            pin("/pinned/a", "A"),
            pin("/pinned/new_1", "New 1"),
            pin("/pinned/new_2", "New 2"),
            pin("/pinned/b", "B"),
        ]
    );
}

#[test]
fn a_gap_falls_back_to_its_surviving_neighbour_and_never_jumps_to_the_end() {
    let places = vec![pin("/pinned/a", "A"), pin("/pinned/b", "B")];
    let a = Location::local("/pinned/a");
    let b = Location::local("/pinned/b");
    let gone = Location::local("/pinned/unpinned-elsewhere");
    for (gap, expected) in [
        ((Some(&a), Some(&b)), Some(1)),
        ((Some(&a), Some(&gone)), Some(1)),
        ((Some(&gone), Some(&b)), Some(1)),
        ((None, Some(&gone)), None),
        ((Some(&gone), Some(&gone)), None),
        ((Some(&gone), None), Some(2)),
        ((None, None), Some(2)),
    ] {
        assert_eq!(pin_gap_index(&places, gap), expected, "{gap:?}");
    }
}

#[test]
fn moving_a_pin_into_an_adjacent_gap_is_a_noop() {
    let original = vec![pin("/a", "A"), pin("/b", "B"), pin("/c", "C")];
    for (source, index, expected) in [
        (1, 1, None),
        (1, 2, None),
        (1, 0, Some(["B", "A", "C"])),
        (0, 3, Some(["B", "C", "A"])),
        (2, 1, Some(["A", "C", "B"])),
    ] {
        let mut places = original.clone();
        assert_eq!(
            move_pinned_place(&mut places, source, index),
            expected.is_some()
        );
        let names = places
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            expected.unwrap_or(["A", "B", "C"]),
            "{source} -> {index}"
        );
    }
}

#[test]
fn pinning_merges_latest_bookmarks_and_persists_aliases_in_selection_order() {
    crate::test_support::gtk_test(
        "ui::window::tests::sidebar::pinning_merges_latest_bookmarks_and_persists_aliases_in_selection_order",
        || {
            use crate::{
                storage::writer,
                test_support::{save_pins, saved_pins},
            };
            use std::{cell::Cell, rc::Rc};
            fn finished(start: impl FnOnce(Rc<Cell<Option<bool>>>)) -> bool {
                let outcome = Rc::new(Cell::new(None));
                start(outcome.clone());
                writer::settle();
                outcome.get().expect("pin edit finished")
            }
            PreferenceManager::seed_saved_preferences_for_test();
            let initial = vec![
                (Location::uri("sftp://host/anchor"), "Anchor".to_owned()),
                (Location::local("/pinned/last"), "Last".to_owned()),
            ];
            save_pins(&initial);
            let view = browser_for_window();
            let sidebar = build_sidebar(view.clone(), PreferenceManager::shared(), false);
            let mut external = initial.clone();
            external.insert(0, (Location::local("/external/pin"), "External".to_owned()));
            save_pins(&external);
            let folders = [
                (Location::local("/mount/folder"), "Mounted".to_owned()),
                (Location::local("/aliases/folder"), "Alias".to_owned()),
                initial[0].clone(),
                (Location::local("/mount/folder"), "Mounted".to_owned()),
            ];
            let locations = folders.clone().map(|(location, _)| location);
            let held = writer::hold();
            let outcome = Rc::new(Cell::new(None));
            let reported = outcome.clone();
            sidebar
                .state
                .pin_folders_into(&folders, (None, Some(&initial[0].0)), move |changed| {
                    reported.set(Some(changed));
                });
            for _ in 0..20 {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert_eq!(outcome.get(), None);
            assert_eq!(saved_pins(), external);
            assert_eq!(*sidebar.state.pinned_places.borrow(), initial);
            drop(held);
            writer::settle();
            assert_eq!(outcome.get(), Some(true));
            external.splice(1..1, folders[..2].iter().cloned());
            assert_eq!(saved_pins(), external);
            assert_eq!(*sidebar.state.pinned_places.borrow(), external);
            assert!(!finished(|outcome| {
                sidebar.state.pin_folders_into(
                    &folders,
                    (Some(&initial[1].0), None),
                    move |changed| outcome.set(Some(changed)),
                );
            }));
            assert_eq!(saved_pins(), external);
            let later = (Location::local("/external/later"), "Later".to_owned());
            external.insert(0, later);
            save_pins(&external);
            assert!(finished(|outcome| {
                sidebar.state.reorder_pinned_into(
                    &locations[0],
                    (Some(&initial[0].0), Some(&initial[1].0)),
                    move |changed| outcome.set(Some(changed)),
                );
            }));
            let moved = external.remove(2);
            external.insert(4, moved);
            assert_eq!(saved_pins(), external);
            external.retain(|(location, _)| location != &locations[0]);
            save_pins(&external);
            assert!(!finished(|outcome| {
                sidebar.state.reorder_pinned_into(
                    &locations[0],
                    (Some(&initial[1].0), None),
                    move |changed| outcome.set(Some(changed)),
                );
            }));
            assert_eq!(saved_pins(), external);
            view.browser().clear_observer();
            sidebar.disconnect();
        },
    );
}
