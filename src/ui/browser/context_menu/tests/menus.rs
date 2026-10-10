// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, MetadataValue};
use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
};
use crate::ui::browser::{BrowserView, PeekBehavior};
use std::time::{Duration, Instant};

pub(super) struct MenuSource;

impl FileSource for MenuSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let parent = crate::adapters::gio_file_for_location(&request.location);
            let entries = [
                "notes.txt",
                "other.txt",
                "run-me",
                "picture.png",
                "archive.zip",
                "archive.rar",
                "folder",
                "photos.zip",
            ]
            .into_iter()
            .map(|name| {
                let in_recent = request.location.is_recent_root() && name == "notes.txt";
                FileEntry {
                    location: if in_recent {
                        Location::local("/fixture/notes.txt")
                    } else {
                        crate::adapters::location_for_file(&parent.child(name)).expect("location")
                    },
                    native_name: name.into(),
                    thumbnail_path: None,
                    display_name: name.into(),
                    kind: if matches!(name, "folder" | "photos.zip") {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    size: MetadataValue::Known(5),
                    modified_unix_seconds: MetadataValue::Known(0),
                    mode: MetadataValue::Known(
                        if matches!(name, "run-me" | "folder" | "photos.zip") {
                            0o755
                        } else {
                            0o644
                        },
                    ),
                    recent_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    image_dimensions: MetadataValue::Unknown,
                    child_count: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                    recent_uri: in_recent.then(|| "recent:///notes.txt".to_owned()),
                }
            })
            .collect();
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries,
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: Some(!is_trash_location(&request.location)),
                can_delete: Some(request.location.uri_value() != Some("trash:///folder")),
            });
        });
        LoadHandle::new(move || task.abort())
    }
}

#[test]
fn dropping_an_action_menu_unparents_its_popover() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::dropping_an_action_menu_unparents_its_popover",
        || {
            let overlay = gtk::Overlay::new();
            let before = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let after = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let menu =
                actions::ActionMenuSection::new(&before, &after, None, overlay.upcast_ref(), None);
            let popover = menu.popover();
            overlay.add_overlay(&popover);
            assert!(popover.parent().is_some());

            drop(menu);

            assert!(popover.parent().is_none());
        },
    );
}

#[test]
fn repeated_navigation_releases_context_menu_bindings() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::repeated_navigation_releases_context_menu_bindings",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let first = tempfile::tempdir().expect("first menu fixture");
                let second = tempfile::tempdir().expect("second menu fixture");
                let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(850)
                    .build();
                window.present();

                let browser = view.browser();
                let navigate = |path: &std::path::Path| {
                    let location = Location::local(path);
                    browser.navigate(location.clone());
                    wait_until(|| {
                        browser.active_location() == Some(location.clone())
                            && browser
                                .column_snapshot(0)
                                .is_some_and(|column| !column.loading)
                    });
                };
                navigate(first.path());
                let retired_menu = (mode == BrowserMode::Icons).then(|| {
                    let menu = open_menu(&view, Some("notes.txt"));
                    let retired = menu.downgrade();
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    retired
                });
                let baseline = manager.listener_count();
                navigate(second.path());
                if let Some(retired) = retired_menu {
                    wait_until(|| retired.upgrade().is_none());
                }
                wait_until(|| manager.listener_count() <= baseline);

                assert_eq!(
                    manager.listener_count(),
                    baseline,
                    "{mode:?} retained context-menu bindings from a retired pane"
                );

                browser.clear_observer();
                window.destroy();
            }
        },
    );
}

pub(super) fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        result.extend(descendants(&current));
    }
    result
}

#[track_caller]
pub(in crate::ui::browser::context_menu) fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "menu fixture did not settle");
        let context = glib::MainContext::default();
        for _ in 0..100 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn label(widget: &gtk::Widget, text: &str) -> Option<gtk::Widget> {
    descendants(widget).into_iter().find(|widget| {
        widget.is_mapped()
            && widget.width() > 0
            && (widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == text)
                || widget
                    .downcast_ref::<gtk::Inscription>()
                    .is_some_and(|label| label.text().as_deref() == Some(text)))
    })
}

pub(super) fn open_menu(view: &BrowserView, name: Option<&str>) -> gtk::Popover {
    let root = view.widget();
    let target = name.map(|name| label(&root, name).expect("mapped entry label"));
    for owner in descendants(&root).into_iter().rev() {
        if !owner.is_mapped() {
            continue;
        }
        let controllers = owner.observe_controllers();
        for index in 0..controllers.n_items() {
            let Some(gesture) = controllers.item(index).and_downcast::<gtk::GestureClick>() else {
                continue;
            };
            if gesture.button() != 3 {
                continue;
            }
            let point = match &target {
                Some(target) if target.is_ancestor(&owner) => target
                    .compute_point(&owner, &gtk::graphene::Point::new(5.0, 5.0))
                    .expect("item coordinates"),
                Some(_) => continue,
                None => gtk::graphene::Point::new(10.0, owner.height() as f32 - 10.0),
            };
            gesture.emit_by_name::<()>(
                "pressed",
                &[&1i32, &f64::from(point.x()), &f64::from(point.y())],
            );
            if let Some(popover) = descendants(&root).into_iter().find_map(|widget| {
                widget
                    .downcast::<gtk::Popover>()
                    .ok()
                    .filter(|popover| popover.is_visible())
            }) {
                wait_until(|| popover.is_mapped());
                let expected = if !view.state.interactive {
                    "chooser-context-menu"
                } else if name.is_some() {
                    "item-context-menu"
                } else {
                    "folder-context-menu"
                };
                if popover.is::<gtk::PopoverMenu>()
                    || descendants(popover.upcast_ref())
                        .iter()
                        .any(|widget| widget.has_css_class(expected))
                {
                    return popover;
                }
                popover.popdown();
                wait_until(|| !popover.is_mapped());
            }
        }
    }
    panic!("no menu for {name:?}");
}

#[test]
fn context_hints_follow_the_active_map() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::context_hints_follow_the_active_map",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            let fixture = tempfile::tempdir().expect("menu fixture");
            let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1000)
                .default_height(850)
                .build();
            window.present();
            view.browser()
                .navigate(crate::model::Location::local(fixture.path()));
            wait_until(|| label(&view.widget(), "notes.txt").is_some());
            manager.set_type_to_search(false);
            let menu = open_menu(&view, Some("notes.txt"));
            let hints = label_texts(&menu);
            assert!(hints.iter().any(|hint| hint == "Space"), "{hints:?}");
            assert!(hints.iter().any(|hint| hint == "Y"), "{hints:?}");
            manager.set_type_to_search(true);
            let hints = label_texts(&menu);
            assert!(
                !hints.iter().any(|hint| hint == "Y"),
                "type-to-search claims y: {hints:?}"
            );
            menu.popdown();
            wait_until(|| !menu.is_mapped());
            manager.set_tenxer_mode(true);
            let menu = open_menu(&view, Some("notes.txt"));
            let hints = label_texts(&menu);
            assert!(!hints.iter().any(|hint| hint == "Space"), "{hints:?}");
            let after = |label: &str| {
                hints
                    .iter()
                    .position(|hint| hint == label)
                    .and_then(|index| hints.get(index + 1))
                    .map(String::as_str)
            };
            for (label, hint) in [
                ("Rename", "R"),
                ("Cut", "X"),
                ("Copy", "Y"),
                ("Move to Trash", "D"),
                ("Permanently delete", "Shift+D"),
                ("Copy path", "Copy name"),
            ] {
                assert_eq!(after(label), Some(hint), "{hints:?}");
            }
            assert!(
                !hints.iter().any(|hint| hint.starts_with("Ctrl+")),
                "{hints:?}"
            );
            assert!(
                hints.iter().any(|hint| hint == "I"),
                "GTK renders the i accelerator as I: {hints:?}"
            );
            menu.popdown();
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn archive_extraction_actions_follow_build_support() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::archive_extraction_actions_follow_build_support",
        || {
            let fixture = tempfile::tempdir().expect("menu fixture");
            let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1000)
                .default_height(850)
                .build();
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| {
                label(&view.widget(), "archive.rar").is_some()
                    && label(&view.widget(), "photos.zip").is_some()
            });
            for (name, supported) in [
                ("archive.zip", true),
                ("archive.rar", cfg!(feature = "rar")),
                ("photos.zip", false),
            ] {
                let menu = open_menu(&view, Some(name));
                let labels = label_texts(&menu);
                if name == "photos.zip" {
                    assert!(
                        labels
                            .iter()
                            .any(|label| label == "Open in Terminal" || label == "Pin to sidebar"),
                        "{labels:?}"
                    );
                }
                for action in ["Extract here", "Extract to…"] {
                    assert_eq!(
                        labels.iter().any(|label| label == action),
                        supported,
                        "{name}: {labels:?}"
                    );
                }
                menu.popdown();
                wait_until(|| !menu.is_mapped());
            }
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn remove_from_recent_is_gated_on_the_recent_location() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::remove_from_recent_is_gated_on_the_recent_location",
        || {
            let fixture = tempfile::tempdir().expect("menu fixture");
            let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1000)
                .default_height(850)
                .build();
            window.present();

            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| label(&view.widget(), "notes.txt").is_some());
            let menu = open_menu(&view, Some("notes.txt"));
            assert!(
                !label_texts(&menu)
                    .iter()
                    .any(|text| text == "Remove from Recent"),
                "{:?}",
                label_texts(&menu)
            );
            menu.popdown();
            wait_until(|| !menu.is_mapped());

            view.browser().navigate(Location::uri("recent:///"));
            wait_until(|| label(&view.widget(), "notes.txt").is_some());
            let menu = open_menu(&view, Some("notes.txt"));
            let labels = label_texts(&menu);
            assert!(
                labels.iter().any(|text| text == "Remove from Recent"),
                "{labels:?}"
            );
            assert!(labels.iter().any(|text| text == "Open"), "{labels:?}");
            assert!(labels.iter().any(|text| text == "Copy"), "{labels:?}");
            menu.popdown();
            wait_until(|| !menu.is_mapped());

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[test]
fn directory_rows_offer_the_open_in_submenu() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::directory_rows_offer_the_open_in_submenu",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            manager.set_type_to_search(false);
            let fixture = tempfile::tempdir().expect("menu fixture");
            let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
            view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1000)
                .default_height(850)
                .build();
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| label(&view.widget(), "folder").is_some());

            let menu = open_menu(&view, Some("folder"));
            let labels = tree_label_texts(&menu);
            for entry in ["Open", "Open in…", "New Tab", "New Window"] {
                assert!(labels.iter().any(|text| text == entry), "{labels:?}");
            }
            for hint in ["Ctrl+Return", "Shift+Return"] {
                assert!(labels.iter().any(|text| text == hint), "{labels:?}");
            }
            menu.popdown();
            wait_until(|| !menu.is_mapped());

            let menu = open_menu(&view, Some("notes.txt"));
            let labels = tree_label_texts(&menu);
            assert!(labels.iter().any(|text| text == "Open"), "{labels:?}");
            for entry in ["Open in…", "New Tab", "New Window"] {
                assert!(!labels.iter().any(|text| text == entry), "{labels:?}");
            }
            menu.popdown();

            view.browser().clear_observer();
            window.destroy();
        },
    );
}

/// Includes collapsed submenu entries, which stay in the widget tree.
fn tree_label_texts(menu: &gtk::Popover) -> Vec<String> {
    descendants(menu.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .filter(|label| !label.text().is_empty())
        .map(|label| label.text().to_string())
        .collect()
}

fn label_texts(menu: &gtk::Popover) -> Vec<String> {
    descendants(menu.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .filter(|label| label.is_visible() && !label.text().is_empty())
        .map(|label| label.text().to_string())
        .collect()
}
