// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, FileEntry, Location, MetadataValue};
use crate::ui::browser::{BrowserView, PeekBehavior};
use std::cell::RefCell;
use std::time::{Duration, Instant};

fn wait_until(condition: impl Fn() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn archive_view(
    destination: &std::path::Path,
) -> (
    BrowserView,
    Rc<crate::app::Browser>,
    gtk::Window,
    gtk::Overlay,
) {
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    browser.navigate(Location::local(destination));
    wait_until(
        || {
            browser
                .column_snapshot(0)
                .is_some_and(|snapshot| !snapshot.loading)
        },
        "archive destination did not load",
    );
    (view, browser, window, overlay)
}

fn progress_layer(overlay: &gtk::Overlay) -> gtk::Box {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.has_css_class("modal-backdrop") {
            return widget.downcast().expect("progress layer");
        }
    }
    panic!("progress layer was not attached");
}

fn has_dissolve_canvas(overlay: &gtk::Overlay) -> bool {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.type_().name() == "StrataDissolveCanvas" {
            return true;
        }
    }
    false
}

#[test]
fn extract_error_needs_password_ignores_quoted_member_names() {
    assert!(extract_error_needs_password("Invalid password"));
    for message in [
        "Unsupported encryption method",
        "A password is required to extract this archive.",
        "The password may be incorrect.",
        "PASSWORD_REQUIRED",
        "Archive member `file.txt`: invalid password",
    ] {
        assert!(extract_error_needs_password(message), "{message}");
    }
    assert!(!extract_error_needs_password(
        "Archive member `passwords.txt` declared 4 bytes but produced more"
    ));
    assert!(!extract_error_needs_password(
        "Archive member `passwords.txt` declared 10 bytes, but only 2 bytes are free at the destination"
    ));
    assert!(!extract_error_needs_password(
        "Not enough free space at the destination to extract `encrypted-notes.md` (0 bytes available)"
    ));
}

#[test]
fn extract_error_needs_password_ignores_backticks_inside_member_names() {
    for name in [
        "note`passwords.txt",
        "a`encrypted`notes.md",
        "`password`",
        "passwords.txt",
    ] {
        for message in [
            format!("Archive member `{name}` declared 4 bytes but produced more"),
            format!("Archive member `{name}` declared 8 bytes but produced 4 bytes"),
            format!(
                "Archive member `{name}` declared 10 bytes, but only 2 bytes are free at the destination"
            ),
            format!(
                "Not enough free space at the destination to extract `{name}` (0 bytes available)"
            ),
        ] {
            assert!(!extract_error_needs_password(&message), "{message}");
        }
    }
}

#[test]
fn successful_delete_dissolves_visible_rows_after_progress_dismissal() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::successful_delete_dissolves_visible_rows_after_progress_dismissal",
        || {
            crate::ui::motion::set_reduce_motion(false);
            gtk::Settings::default()
                .expect("GTK settings")
                .set_gtk_enable_animations(true);
            let directory = tempfile::tempdir().expect("delete directory");
            let path = directory.path().join("visible.txt");
            std::fs::write(&path, "visible").expect("delete fixture");
            let (view, browser, window, overlay) = archive_view(directory.path());
            let state = &view.state;
            let entry = FileEntry {
                location: Location::local(&path),
                native_name: std::ffi::OsString::from("visible.txt"),
                thumbnail_path: None,
                display_name: "visible.txt".into(),
                kind: EntryKind::File,
                size: MetadataValue::Known(7),
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
            };
            let dissolve = RefCell::new(None);
            wait_until(
                || {
                    if dissolve.borrow().is_none() {
                        dissolve.replace(super::super::dissolve_delete::prepare_dissolve(
                            state.overlay.upcast_ref(),
                            std::slice::from_ref(&entry),
                        ));
                    }
                    dissolve.borrow().is_some()
                },
                "visible row was not ready to snapshot",
            );
            state.pending_delete_dissolve.replace(dissolve.into_inner());
            state.show_file_operation_progress(
                16,
                crate::assets::icons::TRASH,
                "Deleting items",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );
            let layer = progress_layer(&overlay);

            state.handle(&BrowserEvent::DeletionFinished { succeeded: true });

            assert!(layer.parent().is_some());
            assert!(!has_dissolve_canvas(&overlay));
            wait_until(
                || layer.parent().is_none(),
                "progress modal did not dismiss",
            );
            wait_until(
                || has_dissolve_canvas(&overlay),
                "dissolve did not start after progress dismissal",
            );
            wait_until(
                || !has_dissolve_canvas(&overlay),
                "dissolve animation did not finish",
            );
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
fn completed_archive_does_not_restore_a_superseded_destination_after_modal_dismissal() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::completed_archive_does_not_restore_a_superseded_destination_after_modal_dismissal",
        || {
            let destination = tempfile::tempdir().expect("archive destination");
            let replacement = tempfile::tempdir().expect("replacement destination");
            std::fs::write(destination.path().join("source.txt"), "source")
                .expect("source fixture");
            let (view, browser, window, overlay) = archive_view(destination.path());
            let state = &view.state;
            state
                .pending_archive_destination
                .replace(Some(Location::local(destination.path())));
            state.show_file_operation_progress(
                16,
                crate::assets::icons::FILE_ARCHIVE,
                "Working",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );
            let layer = progress_layer(&overlay);

            state.handle(&BrowserEvent::ArchiveCompleted {
                select_name: "source.zip".to_owned(),
            });
            assert!(state.pending_select.borrow().is_empty());
            let replacement_location = Location::local(replacement.path());
            browser.navigate(replacement_location.clone());

            wait_until(
                || layer.parent().is_none(),
                "progress modal did not dismiss",
            );
            wait_until(
                || {
                    browser.active_location().as_ref() == Some(&replacement_location)
                        && browser
                            .column_snapshot(0)
                            .is_some_and(|snapshot| !snapshot.loading)
                },
                "replacement destination was not retained",
            );
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(browser.active_location(), Some(replacement_location));
            assert!(state.pending_select.borrow().is_empty());
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
fn completed_extract_does_not_restore_destination_after_navigation_during_modal_dismissal() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::completed_extract_does_not_restore_destination_after_navigation_during_modal_dismissal",
        || {
            let origin = tempfile::tempdir().expect("extract origin");
            let destination = tempfile::tempdir().expect("extract destination");
            let replacement = tempfile::tempdir().expect("replacement destination");
            let (view, browser, window, overlay) = archive_view(origin.path());
            let state = &view.state;
            state
                .pending_navigate
                .replace(Some(Location::local(destination.path())));
            state.show_file_operation_progress(
                16,
                crate::assets::icons::FILE_ARCHIVE,
                "Working",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );
            let layer = progress_layer(&overlay);

            state.handle(&BrowserEvent::ArchiveCompleted {
                select_name: "extracted.txt".to_owned(),
            });
            assert!(state.pending_select.borrow().is_empty());
            let replacement_location = Location::local(replacement.path());
            browser.navigate(replacement_location.clone());

            wait_until(
                || layer.parent().is_none(),
                "progress modal did not dismiss",
            );
            wait_until(
                || {
                    browser.active_location().as_ref() == Some(&replacement_location)
                        && browser
                            .column_snapshot(0)
                            .is_some_and(|snapshot| !snapshot.loading)
                },
                "replacement destination was not retained",
            );
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(browser.active_location(), Some(replacement_location));
            assert!(state.pending_select.borrow().is_empty());
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
fn completed_archive_selects_the_authoritative_model_only_after_modal_dismissal() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::completed_archive_selects_the_authoritative_model_only_after_modal_dismissal",
        || {
            let destination = tempfile::tempdir().expect("archive destination");
            std::fs::write(destination.path().join("alpha.txt"), "source").expect("source fixture");
            std::fs::write(destination.path().join("source.zip"), "archive")
                .expect("archive fixture");
            let (view, browser, window, overlay) = archive_view(destination.path());
            let state = &view.state;
            state
                .pending_archive_destination
                .replace(Some(Location::local(destination.path())));
            state.show_file_operation_progress(
                16,
                crate::assets::icons::FILE_ARCHIVE,
                "Working",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );
            let layer = progress_layer(&overlay);

            state.handle(&BrowserEvent::ArchiveCompleted {
                select_name: "source.zip".to_owned(),
            });
            assert!(state.pending_select.borrow().is_empty());
            assert!(layer.parent().is_some());

            wait_until(
                || {
                    layer.parent().is_none()
                        && browser
                            .focused_entry()
                            .is_some_and(|entry| entry.display_name == "source.zip")
                },
                "archive was not selected after terminal dismissal",
            );
            assert!(state.pending_archive_destination.borrow().is_none());
            assert!(state.pending_select.borrow().is_empty());
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
fn cancelled_archive_clears_flag_and_following_load_restores_transfer_selection() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::cancelled_archive_clears_flag_and_following_load_restores_transfer_selection",
        || {
            let destination = tempfile::tempdir().expect("archive destination");
            std::fs::write(destination.path().join("pasted.txt"), "pasted").expect("paste fixture");
            let (view, browser, window, _overlay) = archive_view(destination.path());
            let state = &view.state;
            state
                .pending_archive_destination
                .replace(Some(Location::local(destination.path())));
            state.show_file_operation_progress(
                16,
                crate::assets::icons::FILE_ARCHIVE,
                "Working",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );

            state.handle(&BrowserEvent::ArchiveCompleted {
                select_name: String::new(),
            });
            assert!(
                state.pending_archive_destination.borrow().is_none(),
                "empty-name completion must clear the archive flag"
            );

            state
                .pending_select
                .borrow_mut()
                .push("pasted.txt".to_owned());
            state.pending_transfer_selection.replace(Some((
                Location::local(destination.path()),
                vec![Location::local(destination.path().join("pasted.txt"))],
            )));
            state.handle(&BrowserEvent::LoadFinished {
                depth: 0,
                truncated: false,
            });
            wait_until(
                || {
                    browser
                        .focused_entry()
                        .is_some_and(|entry| entry.display_name == "pasted.txt")
                },
                "transfer selection was not restored after LoadFinished",
            );
            assert!(state.pending_archive_destination.borrow().is_none());
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
fn operation_cancelled_clears_archive_flag() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::operation_cancelled_clears_archive_flag",
        || {
            let destination = tempfile::tempdir().expect("archive destination");
            let (view, browser, window, _overlay) = archive_view(destination.path());
            let state = &view.state;
            state
                .pending_archive_destination
                .replace(Some(Location::local(destination.path())));

            state.handle(&BrowserEvent::OperationCancelled {
                completed: 0,
                failed: 0,
                not_attempted: 0,
                affected_locations: std::collections::HashSet::new(),
            });
            assert!(
                state.pending_archive_destination.borrow().is_none(),
                "OperationCancelled must clear the archive flag"
            );
            window.destroy();
            browser.clear_observer();
        },
    );
}

#[test]
fn operation_failed_password_prompt_drops_pending_navigate_for_later_completion() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::operation_failed_password_prompt_drops_pending_navigate_for_later_completion",
        || {
            let origin = tempfile::tempdir().expect("extract origin");
            let leftover = tempfile::tempdir().expect("leftover destination");
            let (view, browser, window, _overlay) = archive_view(origin.path());
            let state = &view.state;
            let entry = FileEntry {
                location: Location::local(origin.path().join("encrypted.7z")),
                thumbnail_path: None,
                native_name: std::ffi::OsString::from("encrypted.7z"),
                display_name: "encrypted.7z".into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
            };
            state
                .pending_extract_retry
                .replace(Some((entry, Location::local(leftover.path()))));
            state
                .pending_navigate
                .replace(Some(Location::local(leftover.path())));

            state.handle(&BrowserEvent::OperationFailed {
                message: "archive is password protected".to_owned(),
            });
            assert!(
                state.pending_navigate.borrow().is_none(),
                "password failure must drop the abandoned Extract to… destination"
            );

            state.handle(&BrowserEvent::ArchiveCompleted {
                select_name: "later.txt".to_owned(),
            });
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(
                browser.active_location(),
                Some(Location::local(origin.path())),
                "later completion must not navigate into the leftover destination"
            );
            window.destroy();
            browser.clear_observer();
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

#[test]
fn password_retry_preserves_extract_here_and_extract_to_navigation_intent() {
    crate::test_support::gtk_test(
        "ui::browser::events::tests::password_retry_preserves_extract_here_and_extract_to_navigation_intent",
        || {
            for navigate in [false, true] {
                let origin = tempfile::tempdir().expect("extract origin");
                let destination = Location::local(origin.path());
                let (view, browser, window, _overlay) = archive_view(origin.path());
                let state = &view.state;
                let entry = FileEntry {
                    location: Location::local(origin.path().join("encrypted.7z")),
                    thumbnail_path: None,
                    native_name: std::ffi::OsString::from("encrypted.7z"),
                    display_name: "encrypted.7z".into(),
                    kind: EntryKind::File,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    is_hidden: false,
                    mode: MetadataValue::Unknown,
                };
                state
                    .pending_extract_retry
                    .replace(Some((entry, destination.clone())));
                state
                    .pending_navigate
                    .replace(navigate.then(|| destination.clone()));
                state.handle(&BrowserEvent::OperationFailed {
                    message: "incorrect password".to_owned(),
                });
                assert!(state.pending_navigate.borrow().is_none());
                let widgets = descendants(window.upcast_ref());
                let password = widgets
                    .iter()
                    .find_map(|widget| widget.clone().downcast::<gtk::PasswordEntry>().ok())
                    .expect("password field");
                password.set_text("secret");
                let confirm = widgets
                    .iter()
                    .filter_map(|widget| widget.clone().downcast::<gtk::Button>().ok())
                    .find(|button| button.label().as_deref() == Some("Extract"))
                    .expect("extract button");
                confirm.emit_clicked();
                assert_eq!(
                    *state.pending_navigate.borrow(),
                    navigate.then(|| destination.clone())
                );
                while glib::MainContext::default().iteration(false) {}
                window.destroy();
                browser.clear_observer();
            }
        },
    );
}
