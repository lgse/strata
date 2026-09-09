// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use crate::ui::browser::{BrowserView, PeekBehavior};
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
