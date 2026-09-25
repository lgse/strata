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

// Before popup, is_visible() includes hidden ancestors; a remote URI exposes
// the regression because Rename is available without Compress.

#[test]
fn context_hints_follow_the_active_map() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::menus::context_hints_follow_the_active_map",
        || {
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_omastrata_mode(false);
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
            let menu = open_menu(&view, Some("notes.txt"));
            let hints = label_texts(&menu);
            assert!(hints.iter().any(|hint| hint == "Space"), "{hints:?}");
            assert!(hints.iter().any(|hint| hint == "Y"), "{hints:?}");
            menu.popdown();
            wait_until(|| !menu.is_mapped());
            manager.set_omastrata_mode(true);
            let menu = open_menu(&view, Some("notes.txt"));
            let hints = label_texts(&menu);
            assert!(!hints.iter().any(|hint| hint == "Space"), "{hints:?}");
            assert!(!hints.iter().any(|hint| hint == "Y"), "{hints:?}");
            assert!(hints.iter().any(|hint| hint == "F2"), "{hints:?}");
            assert!(hints.iter().any(|hint| hint == "Ctrl+C"), "{hints:?}");
            menu.popdown();
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

fn label_texts(menu: &gtk::Popover) -> Vec<String> {
    descendants(menu.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .filter(|label| label.is_visible() && !label.text().is_empty())
        .map(|label| label.text().to_string())
        .collect()
}
