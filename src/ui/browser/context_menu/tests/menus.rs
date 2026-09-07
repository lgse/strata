// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::model::{EntryKind, MetadataValue};
use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
};
use crate::ui::browser::{BrowserView, PeekBehavior};
use std::time::{Duration, Instant};

struct MenuSource;

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
                "picture.png",
                "archive.zip",
                "folder",
            ]
            .into_iter()
            .map(|name| FileEntry {
                location: crate::adapters::location_for_file(&parent.child(name))
                    .expect("location"),
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
                mode: MetadataValue::Known(0o644),
                is_hidden: false,
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

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        result.extend(descendants(&current));
    }
    result
}

#[track_caller]
fn wait_until(condition: impl Fn() -> bool) {
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

fn label(widget: &gtk::Widget, text: &str) -> Option<gtk::Widget> {
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

fn open_menu(view: &BrowserView, name: Option<&str>) -> gtk::Popover {
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
                let expected = if name.is_some() {
                    "item-context-menu"
                } else {
                    "folder-context-menu"
                };
                if descendants(popover.upcast_ref())
                    .iter()
                    .any(|widget| widget.has_css_class(expected))
                {
                    return popover;
                }
                popover.popdown();
                wait_until(|| popover.parent().is_none());
            }
        }
    }
    panic!("no menu for {name:?}");
}

fn menu_labels(popover: &gtk::Popover) -> Vec<String> {
    descendants(popover.upcast_ref())
        .iter()
        .filter_map(|widget| {
            let button = widget.downcast_ref::<gtk::Button>()?;
            if !button.is_visible() || !button.is_mapped() {
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
                            "Copy path",
                            "Copy to…",
                            "Quick preview",
                            "Print",
                            "Properties",
                        ],
                        &["Extract here", "Extract to…"],
                    );
                    if in_trash {
                        assert_actions(&menu, &[], &["Rename", "Compress…", "Customize…"]);
                        assert!(!view.begin_rename());
                        assert!(!view.duplicate_selection());
                    } else {
                        assert_actions(
                            &menu,
                            &["Rename", "Compress…", "Move to Trash"],
                            &["Restore"],
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
                    wait_until(|| menu.parent().is_none());
                    view.select_all();
                    let menu = open_menu(&view, Some("notes.txt"));
                    capture_menu(&menu, &format!("{mode:?}-{place}-multiple"));
                    assert_actions(
                        &menu,
                        &["Copy", "Copy paths", "Copy to…"],
                        &["Rename", "Print"],
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
                    wait_until(|| menu.parent().is_none());
                    view.browser().select(0, 0);
                    let menu = open_menu(&view, Some("picture.png"));
                    if in_trash {
                        assert_actions(&menu, &[], &["Print", "Quick preview"]);
                    } else {
                        assert_actions(&menu, &["Print", "Quick preview"], &[]);
                    }
                    menu.popdown();
                    wait_until(|| menu.parent().is_none());
                    let menu = open_menu(&view, Some("archive.zip"));
                    if in_trash {
                        assert_actions(&menu, &[], &["Extract here", "Extract to…"]);
                    } else {
                        assert_actions(&menu, &["Extract here", "Extract to…"], &[]);
                    }
                    menu.popdown();
                    wait_until(|| menu.parent().is_none());
                    let menu = open_menu(&view, None);
                    capture_menu(&menu, &format!("{mode:?}-{place}-blank"));
                    assert_actions(&menu, &["Select All", "Refresh", "Properties"], &[]);
                    if in_trash {
                        assert_actions(
                            &menu,
                            &[],
                            &["New Folder", "New File", "Paste", "Open in Terminal"],
                        );
                    } else {
                        assert_actions(
                            &menu,
                            &["New Folder", "New File", "Paste", "Open in Terminal"],
                            &[],
                        );
                    }
                    menu.popdown();
                    wait_until(|| menu.parent().is_none());
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
    wait_until(|| menu.parent().is_none());
    view.browser().clear_observer();
    window.destroy();
}

fn vertical_offset(menu: &gtk::Popover, widget: &gtk::Widget) -> f32 {
    widget
        .compute_point(menu, &gtk::graphene::Point::new(0.0, 0.0))
        .expect("menu coordinates")
        .y()
}
