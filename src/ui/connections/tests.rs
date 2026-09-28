// SPDX-License-Identifier: MIT

use std::time::Instant;

use super::*;
use crate::services::connections::{ConnectionDraft, connections_path};

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn host_window() -> (gtk::Window, gtk::Overlay, gtk::Label) {
    let anchor = gtk::Label::new(None);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&anchor));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    (window, overlay, anchor)
}

fn descendants<T: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut found = Vec::new();
    let mut stack = vec![root.as_ref().clone()];
    while let Some(widget) = stack.pop() {
        if let Ok(matched) = widget.clone().downcast::<T>() {
            found.push(matched);
        }
        let mut child = widget.last_child();
        while let Some(current) = child {
            child = current.prev_sibling();
            stack.push(current);
        }
    }
    found
}

fn editor(overlay: &gtk::Overlay) -> Option<gtk::Widget> {
    descendants::<gtk::Box>(overlay)
        .into_iter()
        .find(|widget| widget.has_css_class("connection-editor"))
        .map(|widget| widget.upcast())
}

fn button(root: &gtk::Widget, label: &str) -> gtk::Button {
    descendants::<gtk::Button>(root)
        .into_iter()
        .find(|button| button.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label:?} button"))
}

fn entry(root: &gtk::Widget, index: usize) -> gtk::Entry {
    descendants::<gtk::Entry>(root)
        .into_iter()
        .nth(index)
        .expect("form entry")
}

fn saved_file() -> String {
    std::fs::read_to_string(connections_path()).unwrap_or_default()
}

fn store_with(uri: &str, name: &str) -> SavedConnection {
    update_connections(|store| {
        store.add(ConnectionDraft {
            name: name.into(),
            destination: RemoteDestination::parse(uri).expect("destination"),
        })
    })
    .expect("saved")
}

#[test]
fn the_add_form_saves_a_sanitized_connection_without_asking_for_a_password() {
    crate::test_support::gtk_test(
        "ui::connections::tests::the_add_form_saves_a_sanitized_connection_without_asking_for_a_password",
        || {
            let (window, overlay, anchor) = host_window();
            let changes = Rc::new(Cell::new(0));
            let counted = changes.clone();
            let _watch = watch_connections(move || counted.set(counted.get() + 1));

            show_connection_editor(&anchor, ConnectionEditor::Add(None));
            let form = editor(&overlay).expect("editor opens");
            assert!(
                descendants::<gtk::PasswordEntry>(&form).is_empty(),
                "connections never store a password"
            );
            descendants::<gtk::ToggleButton>(&form)
                .into_iter()
                .find(|toggle| toggle.label().as_deref() == Some("SFTP"))
                .expect("SFTP option")
                .set_active(true);
            entry(&form, 0).set_text("Files.Example.COM");
            entry(&form, 1).set_text("2222");
            entry(&form, 2).set_text("alice");
            entry(&form, 4).set_text("/srv/data/");
            assert_eq!(
                entry(&form, 5).placeholder_text().as_deref(),
                Some("data on files.example.com"),
                "the name defaults from the destination"
            );
            button(&form, "Save").emit_clicked();

            wait_until("the form to close", || editor(&overlay).is_none());
            assert_eq!(changes.get(), 1, "open sidebars are told to rebuild");
            let saved = saved_connections().connections();
            assert_eq!(saved.len(), 1);
            assert_eq!(saved[0].name, "data on files.example.com");
            assert_eq!(
                saved[0].destination().canonical_uri(),
                "sftp://alice@files.example.com:2222/srv/data"
            );
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(connections_path())
                .expect("connection file")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "the connection file is private");
            window.destroy();
        },
    );
}

#[test]
fn duplicate_and_invalid_forms_explain_the_problem_and_keep_the_file() {
    crate::test_support::gtk_test(
        "ui::connections::tests::duplicate_and_invalid_forms_explain_the_problem_and_keep_the_file",
        || {
            let (window, overlay, anchor) = host_window();
            store_with("smb://nas/media", "Media");
            let before = saved_file();

            show_connection_editor(&anchor, ConnectionEditor::Add(None));
            let form = editor(&overlay).expect("editor opens");
            entry(&form, 0).set_text("NAS");
            button(&form, "Save").emit_clicked();
            let errors = |form: &gtk::Widget| {
                descendants::<gtk::Label>(form)
                    .into_iter()
                    .filter(|label| label.has_css_class("form-field-error") && label.is_visible())
                    .map(|label| label.text().to_string())
                    .collect::<Vec<_>>()
            };
            assert_eq!(errors(&form), ["Enter a share name."]);

            entry(&form, 3).set_text("MEDIA");
            button(&form, "Save").emit_clicked();
            assert_eq!(errors(&form), ["“Media” already connects there."]);
            assert!(editor(&overlay).is_some(), "the form stays open to fix it");
            assert_eq!(saved_file(), before);
            window.destroy();
        },
    );
}

#[test]
fn rename_changes_only_the_label() {
    crate::test_support::gtk_test(
        "ui::connections::tests::rename_changes_only_the_label",
        || {
            let (window, overlay, anchor) = host_window();
            let saved = store_with("davs://cloud/dav", "Cloud");
            show_rename_connection(&anchor, saved.clone());
            let dialog: gtk::Widget = overlay.last_child().expect("rename dialog");
            let name = entry(&dialog, 0);
            assert_eq!(name.text(), "Cloud");
            name.set_text("Work files");
            button(&dialog, "Rename").emit_clicked();
            let connections = saved_connections().connections();
            assert_eq!(connections[0].name, "Work files");
            assert_eq!(connections[0].destination(), saved.destination());
            window.destroy();
        },
    );
}

#[test]
fn the_save_offer_appears_only_for_unsaved_remote_destinations() {
    crate::test_support::gtk_test(
        "ui::connections::tests::the_save_offer_appears_only_for_unsaved_remote_destinations",
        || {
            let (window, overlay, _) = host_window();
            let offer = |overlay: &gtk::Overlay| {
                descendants::<gtk::Box>(overlay)
                    .into_iter()
                    .find(|widget| widget.has_css_class("connection-save-offer"))
            };
            offer_to_save(&overlay, &Location::local("/tmp"));
            assert!(offer(&overlay).is_none());

            store_with("sftp://host/srv", "Server");
            offer_to_save(&overlay, &Location::uri("sftp://host/srv/nested"));
            assert!(offer(&overlay).is_none(), "already saved");

            offer_to_save(&overlay, &Location::uri("ftps://bob@other:2121/pub"));
            let banner = offer(&overlay).expect("save offer");
            assert!(
                saved_connections().connections().len() == 1,
                "nothing is saved yet"
            );
            button(banner.upcast_ref(), "Save Connection…").emit_clicked();
            assert!(offer(&overlay).is_none());
            let form = editor(&overlay).expect("prefilled editor");
            assert_eq!(entry(&form, 0).text(), "other");
            assert_eq!(entry(&form, 1).text(), "2121");
            assert_eq!(entry(&form, 2).text(), "bob");
            assert_eq!(entry(&form, 4).text(), "/pub");
            window.destroy();
        },
    );
}
