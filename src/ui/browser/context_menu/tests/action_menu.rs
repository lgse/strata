// SPDX-License-Identifier: MIT

use std::fs;
use std::rc::Rc;

use super::menus::{descendants, label, open_menu, wait_until};
use super::*;
use crate::model::{EntryKind, MetadataValue};
use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
};
use crate::ui::browser::{BrowserView, PeekBehavior};

const ALWAYS: &str = r#"
schema_version = 1
id = "always"
name = "Always available"
menu = "top"

[when]

[run]
runtime = "command"
program = "true"
args = ["{paths}"]
"#;

const PNG_ONLY: &str = r#"
schema_version = 1
id = "png-only"
name = "PNG only"
menu = "submenu"

[when]
kinds = ["file"]
extensions = ["png"]

[run]
runtime = "command"
program = "true"
args = ["{paths}"]
"#;

const MISSING_TOOL: &str = r#"
schema_version = 1
id = "missing-tool"
name = "Missing tool"
menu = "top"

[when]

[run]
runtime = "command"
program = "definitely-not-a-real-program-xyz"
"#;

fn write_action(directory: &std::path::Path, id: &str, manifest: &str) {
    let action_directory = directory.join(id);
    fs::create_dir_all(&action_directory).expect("action directory");
    fs::write(action_directory.join("action.toml"), manifest).expect("manifest");
}

fn button_text(button: &gtk::Button) -> Option<String> {
    descendants(button.upcast_ref())
        .iter()
        .find_map(|child| child.downcast_ref::<gtk::Label>())
        .map(|label| label.text().to_string())
}

fn action_buttons(widget: &gtk::Widget) -> Vec<String> {
    descendants(widget)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .filter(|button| button.is_mapped() && button.is_sensitive())
        .filter_map(|button| button_text(&button))
        .collect()
}

fn button(widget: &gtk::Widget, text: &str) -> Option<gtk::Button> {
    descendants(widget)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.is_mapped() && button_text(button).as_deref() == Some(text))
}

fn insensitive_button_names(widget: &gtk::Widget) -> Vec<String> {
    descendants(widget)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .filter(|button| button.is_mapped() && !button.is_sensitive())
        .filter_map(|button| button_text(&button))
        .collect()
}

fn open_view(location: Location) -> (BrowserView, gtk::Window) {
    let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(1000)
        .default_height(850)
        .build();
    window.present();
    view.browser().navigate(location);
    (view, window)
}

#[test]
fn custom_actions_appear_for_matching_items_and_run_through_the_job_service() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::action_menu::custom_actions_appear_for_matching_items_and_run_through_the_job_service",
        || {
            let actions = crate::storage::config_directory().join("actions");
            fs::create_dir_all(&actions).expect("actions directory");
            write_action(&actions, "always", ALWAYS);
            write_action(&actions, "png-only", PNG_ONLY);
            write_action(&actions, "missing-tool", MISSING_TOOL);
            fs::write(actions.join("notes.txt"), "not an action").expect("stray file");
            write_action(&actions, "broken", "schema_version = nope\n");

            let fixture = tempfile::tempdir().expect("fixture");
            let (view, window) = open_view(Location::local(fixture.path()));
            wait_until(|| label(&view.widget(), "notes.txt").is_some());

            let menu = open_menu(&view, Some("notes.txt"));
            let labels = action_buttons(menu.upcast_ref());
            assert!(
                labels.iter().any(|label| label == "Always available"),
                "the unconditional action is offered: {labels:?}"
            );
            assert!(
                !labels.iter().any(|label| label == "PNG only"),
                "an extension rule keeps the action out of this selection: {labels:?}"
            );
            assert!(
                insensitive_button_names(menu.upcast_ref())
                    .iter()
                    .any(|label| label == "Missing tool"),
                "an action that cannot run stays visible but disabled"
            );

            let jobs = crate::ui::jobs::shared();
            let before = jobs.snapshot().len();
            let mut confirmed = crate::ui::actions::shared()
                .catalog()
                .get("always")
                .expect("action")
                .as_ref()
                .clone();
            confirmed.definition.run.confirm = true;
            crate::ui::actions::run_action(
                &gtk::Button::new(),
                Rc::new(confirmed),
                vec![fixture.path().join("notes.txt")],
                fixture.path().to_owned(),
                crate::services::InvocationSource::Selection,
            );
            assert_eq!(
                jobs.snapshot().len(),
                before,
                "no execution without a confirmation host"
            );
            button(menu.upcast_ref(), "Always available")
                .expect("action button")
                .emit_clicked();
            wait_until(|| jobs.snapshot().len() > before);
            assert!(
                jobs.snapshot()
                    .iter()
                    .any(|job| job.action_name == "Always available"),
                "the clicked action becomes a job: {:?}",
                jobs.snapshot()
            );
            jobs.clear_finished();
            menu.popdown();
            wait_until(|| menu.parent().is_none());

            let menu = open_menu(&view, Some("picture.png"));
            assert!(
                action_buttons(menu.upcast_ref())
                    .iter()
                    .any(|label| label == "Always available"),
                "top-level actions stay in the menu body"
            );
            let actions_button = button(menu.upcast_ref(), "Actions").expect("actions submenu");
            actions_button.emit_clicked();
            let submenu = descendants(menu.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Popover>().ok())
                .filter(|popover| popover.as_ptr() != menu.as_ptr())
                .find(|popover| {
                    popover.is_visible()
                        && action_buttons(popover.upcast_ref())
                            .iter()
                            .any(|label| label == "PNG only")
                })
                .expect("the submenu lists the matching action");
            assert!(
                submenu
                    .parent()
                    .is_some_and(|parent| parent.is::<gtk::Button>()),
                "the submenu is a popover anchored to its menu row"
            );
            menu.popdown();
            wait_until(|| menu.parent().is_none());
            wait_until(|| !submenu.is_visible());

            let menu = open_menu(&view, Some("folder"));
            assert!(
                action_buttons(menu.upcast_ref())
                    .iter()
                    .any(|label| label == "Always available"),
                "folder backgrounds offer applicable actions"
            );
            menu.popdown();
            wait_until(|| menu.parent().is_none());
            drop(window);

            let (trash, trash_window) = open_view(Location::uri("trash:///"));
            wait_until(|| label(&trash.widget(), "notes.txt").is_some());
            let menu = open_menu(&trash, Some("notes.txt"));
            assert!(
                !action_buttons(menu.upcast_ref())
                    .iter()
                    .any(|label| label == "Always available"),
                "custom actions are not offered for non-native locations"
            );
            menu.popdown();
            wait_until(|| menu.parent().is_none());
            drop(trash_window);
        },
    );
}

struct MenuSource;

impl FileSource for MenuSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let task = glib::MainContext::default().spawn_local(async move {
            let parent = crate::adapters::gio_file_for_location(&request.location);
            let entries = ["notes.txt", "picture.png", "folder"]
                .into_iter()
                .map(|name| {
                    let location =
                        crate::adapters::location_for_file(&parent.child(name)).expect("location");
                    crate::model::FileEntry {
                        location,
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
                        recent_unix_seconds: MetadataValue::Unknown,
                        is_hidden: false,
                        mode: MetadataValue::Known(if name == "folder" { 0o755 } else { 0o644 }),
                        image_dimensions: MetadataValue::Unknown,
                        child_count: MetadataValue::Unknown,
                        duration_seconds: MetadataValue::Unknown,
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
                can_trash: Some(true),
                can_delete: Some(true),
            });
        });
        LoadHandle::new(move || task.abort())
    }
}
