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
            ]
            .into_iter()
            .map(|name| FileEntry {
                location: if request.location.is_recent_root() && name == "notes.txt" {
                    Location::local("/fixture/notes.txt")
                } else {
                    crate::adapters::location_for_file(&parent.child(name)).expect("location")
                },
                native_name: name.into(),
                thumbnail_path: None,
                display_name: name.into(),
                kind: if name == "folder" {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: MetadataValue::Known(5),
                modified_unix_seconds: MetadataValue::Known(0),
                mode: MetadataValue::Known(if matches!(name, "run-me" | "folder") {
                    0o755
                } else {
                    0o644
                }),
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
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
fn run_is_only_offered_for_one_regular_executable_file() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::run_is_only_offered_for_one_regular_executable_file",
        || {
            let fixture = tempfile::tempdir().expect("normal directory fixture");
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(850)
                    .build();
                window.present();
                view.browser().navigate(Location::local(fixture.path()));
                wait_until(|| label(&view.widget(), "run-me").is_some());

                let menu = open_menu(&view, Some("run-me"));
                assert_actions(&menu, &["Open", "Open With…", "Run"], &[]);
                button_with_label(menu.upcast_ref(), "Run").activate();
                wait_until(|| label(&view.widget(), "Run this program?").is_some());
                button_with_label(&view.widget(), "Cancel").activate();
                wait_until(|| label(&view.widget(), "Run this program?").is_none());

                let menu = open_menu(&view, Some("notes.txt"));
                assert_actions(&menu, &[], &["Run", "Open file location"]);
                menu.popdown();
                wait_until(|| !menu.is_mapped());

                let menu = open_menu(&view, Some("folder"));
                assert_actions(&menu, &[], &["Run", "Open file location"]);
                menu.popdown();
                wait_until(|| !menu.is_mapped());

                view.select_all();
                let menu = open_menu(&view, Some("run-me"));
                assert_actions(&menu, &[], &["Run", "Open file location"]);
                menu.popdown();
                wait_until(|| !menu.is_mapped());
            }
        },
    );
}

#[test]
fn run_is_offered_for_an_executable_inline_search_result() {
    use std::os::unix::fs::PermissionsExt;

    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::run_is_offered_for_an_executable_inline_search_result",
        || {
            let fixture = tempfile::tempdir().expect("search fixture");
            let program = fixture.path().join("run-search-result");
            std::fs::write(&program, b"#!/bin/sh\n").expect("program fixture");
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("executable permissions");

            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(850)
                    .build();
                window.present();
                view.browser().navigate(Location::local(fixture.path()));
                wait_until(|| label(&view.widget(), "run-search-result").is_some());
                assert!(view.show_filter_with_query("run-search-result"));
                wait_until(|| {
                    descendants(&view.widget())
                        .iter()
                        .any(|widget| widget.is_mapped() && widget.has_css_class("filter-result"))
                });
                wait_until(|| label(&view.widget(), "run-search-result").is_some());

                let menu = open_menu(&view, Some("run-search-result"));
                assert_actions(&menu, &["Run"], &[]);
                menu.popdown();
                wait_until(|| !menu.is_mapped());
                view.browser().clear_observer();
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
pub(super) fn wait_until(condition: impl Fn() -> bool) {
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

fn menu_labels(popover: &gtk::Popover) -> Vec<String> {
    descendants(popover.upcast_ref())
        .iter()
        .filter_map(|widget| {
            if !(widget.is::<gtk::Button>()
                || widget.accessible_role() == gtk::AccessibleRole::MenuItem)
                || !widget.is_visible()
                || !widget.is_mapped()
            {
                return None;
            }
            descendants(widget).iter().find_map(|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .map(|label| label.text().to_string())
            })
        })
        .collect()
}

fn button_with_label(widget: &gtk::Widget, text: &str) -> gtk::Widget {
    descendants(widget)
        .into_iter()
        .filter(|widget| {
            widget.is::<gtk::Button>() || widget.accessible_role() == gtk::AccessibleRole::MenuItem
        })
        .find(|button| {
            descendants(button).iter().any(|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == text)
            })
        })
        .unwrap_or_else(|| panic!("missing {text} button"))
}

fn assert_actions(popover: &gtk::Popover, present: &[&str], absent: &[&str]) {
    let labels = menu_labels(popover);
    for name in present {
        assert!(
            labels.iter().any(|label| label == name),
            "missing {name}: {labels:?}"
        );
    }
    for name in absent {
        assert!(
            !labels.iter().any(|label| label == name),
            "unsupported {name}: {labels:?}"
        );
    }
}

fn assert_separators_divide_actions(popover: &gtk::Popover) {
    let rendered: Vec<bool> = descendants(&popover.clone().upcast::<gtk::Widget>())
        .into_iter()
        .filter(|widget| widget.is_mapped())
        .filter_map(|widget| {
            if widget.downcast_ref::<gtk::Separator>().is_some() {
                Some(true)
            } else if widget.downcast_ref::<gtk::Label>().is_some() {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    assert_ne!(
        rendered.last(),
        Some(&true),
        "menu ends with a separator and no following action"
    );
    assert!(
        !rendered.windows(2).any(|pair| pair == [true, true]),
        "menu renders two separators with no action between them"
    );
}

fn capture_menu(menu: &gtk::Popover, name: &str) {
    let Some(output) = std::env::var_os("STRATA_TRASH_MENU_VISUALS") else {
        return;
    };
    let output = std::path::PathBuf::from(output);
    std::fs::create_dir_all(&output).expect("visual evidence directory");
    wait_until(|| menu.width() > 0 && menu.height() > 0);
    let node = RefCell::new(None);
    wait_until(|| {
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(menu)).snapshot(
            &snapshot,
            f64::from(menu.width()),
            f64::from(menu.height()),
        );
        node.replace(snapshot.to_node());
        node.borrow().is_some()
    });
    menu.renderer()
        .expect("menu renderer")
        .render_texture(node.into_inner().expect("menu render node"), None)
        .save_to_png(output.join(format!("{name}.png")))
        .expect("save menu evidence");
}

#[test]
fn menus_and_keyboard_actions_follow_supported_operations_in_every_mode() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::menus_and_keyboard_actions_follow_supported_operations_in_every_mode",
        || {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(include_str!("../../../../style.css"));
            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().expect("display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let fixture = tempfile::tempdir().expect("normal directory fixture");
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                for location in [
                    Location::uri("trash:///"),
                    Location::uri("trash:///folder"),
                    Location::local(fixture.path()),
                ] {
                    let in_trash = is_trash_location(&location);
                    let nested = location.uri_value() == Some("trash:///folder");
                    let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
                    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
                    view.set_view_mode(mode);
                    let window = gtk::Window::builder()
                        .child(&view.widget())
                        .default_width(1000)
                        .default_height(850)
                        .build();
                    window.present();
                    view.browser().navigate(location.clone());
                    eprintln!("checking {mode:?} {}", location.display_path());
                    wait_until(|| label(&view.widget(), "notes.txt").is_some());
                    let menu = open_menu(&view, Some("notes.txt"));
                    let place = if nested {
                        "nested-trash"
                    } else if in_trash {
                        "trash"
                    } else {
                        "normal"
                    };
                    capture_menu(&menu, &format!("{mode:?}-{place}-single"));
                    assert_actions(
                        &menu,
                        &[
                            "Open",
                            "Copy",
                            "Duplicate",
                            "Copy path",
                            "Copy to…",
                            "Quick preview",
                            "Print",
                            "Properties",
                        ],
                        &["Extract here", "Extract to…"],
                    );
                    if in_trash {
                        assert_actions(
                            &menu,
                            &[],
                            &["Rename", "Compress…", "Customize…", "Open file location"],
                        );
                        assert!(!view.begin_rename());
                        assert!(!view.duplicate_selection());
                    } else {
                        assert_actions(
                            &menu,
                            &["Rename", "Compress…", "Move to Trash"],
                            &["Restore", "Open file location"],
                        );
                    }
                    if nested {
                        assert_actions(
                            &menu,
                            &[],
                            &["Restore", "Permanently delete", "Cut", "Move to…"],
                        );
                        assert!(!view.cut_selection());
                    } else {
                        assert_actions(&menu, &["Cut", "Move to…", "Permanently delete"], &[]);
                        assert!(view.cut_selection());
                        assert!(view.copy_selection());
                        if in_trash {
                            assert_actions(&menu, &["Restore"], &[]);
                        }
                    }
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    view.select_all();
                    let menu = open_menu(&view, Some("notes.txt"));
                    capture_menu(&menu, &format!("{mode:?}-{place}-multiple"));
                    assert_actions(
                        &menu,
                        &["Copy", "Duplicate", "Copy paths", "Copy to…", "Properties"],
                        &["Rename", "Print", "Open file location"],
                    );
                    if in_trash {
                        assert_actions(&menu, &[], &["Compress…"]);
                    }
                    if nested {
                        assert_actions(
                            &menu,
                            &[],
                            &["Restore items", "Permanently delete", "Cut", "Move to…"],
                        );
                        assert!(!view.cut_selection());
                    } else {
                        assert_actions(&menu, &["Cut", "Move to…", "Permanently delete"], &[]);
                        if in_trash {
                            assert_actions(&menu, &["Restore items"], &[]);
                        }
                    }
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    view.browser().select(0, 0);
                    let menu = open_menu(&view, Some("picture.png"));
                    if in_trash {
                        assert_actions(&menu, &["Quick preview"], &["Print"]);
                    } else {
                        assert_actions(&menu, &["Print", "Quick preview"], &[]);
                    }
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    let menu = open_menu(&view, Some("archive.zip"));
                    if in_trash {
                        assert_actions(&menu, &[], &["Extract here", "Extract to…"]);
                    } else {
                        assert_actions(&menu, &["Extract here", "Extract to…"], &[]);
                    }
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    let menu = open_menu(&view, Some("archive.rar"));
                    if in_trash {
                        assert_actions(&menu, &[], &["Extract here", "Extract to…"]);
                    } else {
                        assert_actions(&menu, &["Extract here", "Extract to…"], &[]);
                    }
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    let menu = open_menu(&view, None);
                    capture_menu(&menu, &format!("{mode:?}-{place}-blank"));
                    assert_actions(&menu, &["Select All", "Refresh", "Properties"], &[]);
                    if in_trash {
                        assert_actions(
                            &menu,
                            &[],
                            &[
                                "New Folder",
                                "New File",
                                "Paste",
                                "Open With…",
                                "Open in Terminal",
                                "Customize…",
                            ],
                        );
                    } else {
                        assert_actions(
                            &menu,
                            &[
                                "New Folder",
                                "New File",
                                "Open With…",
                                "Paste",
                                "Open in Terminal",
                                "Customize…",
                            ],
                            &[],
                        );
                    }
                    menu.popdown();
                    wait_until(|| !menu.is_mapped());
                    view.create_new_folder();
                    if in_trash {
                        assert!(!view.new_entry_is_active());
                    } else {
                        wait_until(|| view.new_entry_is_active());
                    }
                    view.cancel_new_entry();
                    view.keyboard_navigation();
                    if mode == BrowserMode::Columns {
                        let columns = view.state.columns.borrow();
                        assert_eq!(columns[0].destination_hint.label().is_empty(), in_trash);
                        assert_eq!(
                            columns[0].shell.has_css_class("destination-column"),
                            !in_trash
                        );
                    }
                    view.browser().clear_observer();
                    window.destroy();
                }
            }
            assert_remote_menu_separates_rename_from_properties();
        },
    );
}

#[test]
fn recent_background_menu_rejects_physical_directory_actions() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::recent_background_menu_rejects_physical_directory_actions",
        || {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(include_str!("../../../../style.css"));
            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().expect("display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(850)
                    .build();
                window.present();
                view.browser().navigate(Location::uri("recent:///"));
                wait_until(|| label(&view.widget(), "notes.txt").is_some());

                let menu = open_menu(&view, None);
                assert_actions(
                    &menu,
                    &["Select All", "Refresh"],
                    &[
                        "New Folder",
                        "New File",
                        "Paste",
                        "Open With…",
                        "Open in Terminal",
                        "Customize…",
                        "Properties",
                    ],
                );
                assert_separators_divide_actions(&menu);
                menu.popdown();
                wait_until(|| !menu.is_mapped());

                view.create_new_folder();
                assert!(!view.new_entry_is_active());
                if mode == BrowserMode::Columns {
                    let columns = view.state.columns.borrow();
                    assert!(columns[0].destination_hint.label().is_empty());
                    assert!(!columns[0].shell.has_css_class("destination-column"));
                }
                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn recent_item_open_file_location_uses_the_target_parent() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::recent_item_open_file_location_uses_the_target_parent",
        || {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(include_str!("../../../../style.css"));
            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().expect("display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(850)
                    .build();
                window.present();
                view.browser().navigate(Location::uri("recent:///"));
                wait_until(|| label(&view.widget(), "notes.txt").is_some());

                let menu = open_menu(&view, Some("notes.txt"));
                assert_actions(&menu, &["Open file location"], &[]);
                button_with_label(menu.upcast_ref(), "Open file location").activate();
                wait_until(|| !menu.is_mapped());
                wait_until(|| {
                    view.browser().active_location().as_ref() == Some(&Location::local("/fixture"))
                });
                wait_until(|| {
                    view.browser().focused_item().is_some_and(|(_, _, entry)| {
                        entry.location == Location::local("/fixture/notes.txt")
                    })
                });

                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}

// Before popup, is_visible() includes hidden ancestors; a remote URI exposes
// the regression because Rename is available without Compress.
fn assert_remote_menu_separates_rename_from_properties() {
    let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(1000)
        .default_height(850)
        .build();
    window.present();
    view.browser()
        .navigate(Location::uri("sftp://example.test/remote"));
    wait_until(|| label(&view.widget(), "notes.txt").is_some());

    let menu = open_menu(&view, Some("notes.txt"));
    assert_actions(&menu, &["Rename", "Properties"], &["Compress…"]);
    wait_until(|| menu.width() > 0 && label(menu.upcast_ref(), "Rename").is_some());
    let rename = vertical_offset(&menu, &label(menu.upcast_ref(), "Rename").expect("rename"));
    let properties = vertical_offset(
        &menu,
        &label(menu.upcast_ref(), "Properties").expect("properties"),
    );
    assert!(
        descendants(menu.upcast_ref())
            .iter()
            .filter(|widget| widget.is::<gtk::Separator>() && widget.is_mapped())
            .any(|separator| {
                let offset = vertical_offset(&menu, separator);
                offset > rename && offset < properties
            }),
        "rename must stay separated from the properties group"
    );

    menu.popdown();
    wait_until(|| !menu.is_mapped());

    let menu = open_menu(&view, None);
    assert_actions(&menu, &["Properties"], &["Customize…"]);
    menu.popdown();
    wait_until(|| !menu.is_mapped());
    view.browser().clear_observer();
    window.destroy();
}

fn vertical_offset(menu: &gtk::Popover, widget: &gtk::Widget) -> f32 {
    widget
        .compute_point(menu, &gtk::graphene::Point::new(0.0, 0.0))
        .expect("menu coordinates")
        .y()
}

#[test]
fn open_file_location_navigates_to_parent_folder_and_selects_file() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::open_file_location_navigates_to_parent_folder_and_selects_file",
        || {
            let fixture = tempfile::tempdir().expect("fixture dir");
            let sub = fixture.path().join("nested_folder");
            std::fs::create_dir(&sub).expect("nested dir");
            let target_file = sub.join("target.pdf");
            std::fs::write(&target_file, b"%PDF-1.4\n").expect("target file");

            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(850)
                    .build();
                window.present();
                view.browser().navigate(Location::local(fixture.path()));
                let direct_file = fixture.path().join("direct.txt");
                std::fs::write(&direct_file, b"direct\n").expect("direct file");
                wait_until(|| label(&view.widget(), "direct.txt").is_some());

                let menu = open_menu(&view, Some("direct.txt"));
                assert_actions(&menu, &[], &["Open file location"]);
                menu.popdown();
                wait_until(|| !menu.is_mapped());

                assert!(view.show_filter_with_query("direct"));
                wait_until(|| label(&view.widget(), "direct.txt").is_some());

                let menu = open_menu(&view, Some("direct.txt"));
                assert_actions(&menu, &["Open file location"], &[]);

                let open_button = button_with_label(menu.upcast_ref(), "Open file location");
                open_button.activate();
                wait_until(|| !menu.is_mapped());
                wait_until(|| label(&view.widget(), "nested_folder").is_some());

                wait_until(|| {
                    view.browser().active_location().as_ref()
                        == Some(&Location::local(fixture.path()))
                });
                wait_until(|| {
                    view.browser()
                        .focused_item()
                        .is_some_and(|(_, _, entry)| entry.display_name == "direct.txt")
                });

                wait_until(|| match mode {
                    BrowserMode::Columns => view.state.columns.borrow()[0]
                        .filter_entry
                        .text()
                        .is_empty(),
                    BrowserMode::Icons | BrowserMode::List => view
                        .state
                        .mode_views
                        .borrow()
                        .capture_active_filter()
                        .query
                        .is_empty(),
                });
                assert!(view.show_filter_with_query("target.pdf"));
                wait_until(|| label(&view.widget(), "target.pdf").is_some());

                let menu = open_menu(&view, Some("target.pdf"));
                assert_actions(&menu, &["Open file location"], &[]);

                let open_button = button_with_label(menu.upcast_ref(), "Open file location");
                open_button.activate();
                wait_until(|| !menu.is_mapped());

                wait_until(|| {
                    view.browser().active_location().as_ref() == Some(&Location::local(&sub))
                });
                wait_until(|| {
                    view.browser()
                        .focused_item()
                        .is_some_and(|(_, _, entry)| entry.display_name == "target.pdf")
                });

                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}
