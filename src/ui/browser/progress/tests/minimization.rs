// SPDX-License-Identifier: MIT

use super::super::*;
use crate::{
    adapters::LocalFileSource,
    model::Location,
    services::{
        ArchiveFormat, FileSource, OperationEvent, OperationRequestId, PasteItem, TransferConflict,
    },
    test_support::operations::{HeldOperations, entry},
    ui::{
        blur::BlurBin,
        browser::{BrowserView, PeekBehavior},
    },
};
use gtk::subclass::prelude::ObjectSubclassIsExt;
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

struct Fixture {
    temp: tempfile::TempDir,
    view: BrowserView,
    window: gtk::Window,
    blur: BlurBin,
    operations: Rc<HeldOperations>,
}

impl Fixture {
    fn new() -> Self {
        crate::ui::prepare_portal_ui();
        set_file_progress_delay_for_test(Duration::ZERO);
        let temp = tempfile::tempdir().expect("progress fixture");
        let source: Rc<dyn FileSource> = Rc::new(LocalFileSource);
        let view = BrowserView::new(source, PeekBehavior::default());
        let operations = Rc::new(HeldOperations::default());
        view.browser().set_operation_provider(operations.clone());
        let blur = BlurBin::new(&view.widget());
        let outer = gtk::Overlay::new();
        outer.set_child(Some(&blur));
        let window = gtk::Window::builder()
            .default_width(1024)
            .default_height(700)
            .child(&outer)
            .build();
        crate::ui::window::install_modal_focus_trap(&window);
        window.present();
        Self {
            temp,
            view,
            window,
            blur,
            operations,
        }
    }
    fn transfer(&self, name: &str, moving: bool) -> OperationRequestId {
        self.view.browser().transfer(
            Location::local(self.temp.path()),
            vec![PasteItem {
                source: Location::local(self.temp.path().join(name)),
                conflict: TransferConflict::FailIfExists,
            }],
            moving,
            false,
        );
        self.view
            .browser()
            .last_started_operation()
            .expect("transfer started")
    }
    fn progress(&self, id: OperationRequestId) -> Rc<FileProgressState> {
        let progress = self
            .view
            .state
            .background_file_progress
            .borrow()
            .get(&id)
            .expect("automatically docked job")
            .progress
            .clone();
        pump_until(|| progress.file_progress_view.borrow().is_some());
        progress
    }
    fn update(&self, id: OperationRequestId, name: &str, bytes: u64) {
        self.operations.emit(
            id,
            OperationEvent::TransferProgress {
                request_id: id,
                completed_items: 0,
                completed_files: 0,
                total_files: Some(1),
                current_file: Some(name.into()),
                transferred_bytes: bytes,
                total_bytes: Some(100),
                created_location: None,
            },
        );
    }
    fn finish(&self, id: OperationRequestId) {
        self.operations.emit(
            id,
            OperationEvent::Pasted {
                request_id: id,
                locations: Vec::new(),
            },
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.view.browser().clear_observer();
        self.view.dispose_file_progress();
        self.view.browser().cancel_background_operations();
        self.window.destroy();
        set_file_progress_delay_for_test(Duration::from_millis(220));
    }
}

fn progress_card(progress: &FileProgressState) -> Rc<crate::ui::progress_dock::CompactProgress> {
    progress
        .file_progress_view
        .borrow()
        .as_ref()
        .expect("visible progress")
        .compact
        .clone()
        .expect("docked progress")
}

fn pump_until(predicate: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        while context.pending() {
            context.iteration(false);
        }
        assert!(Instant::now() < deadline, "progress transition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn copy_starts_in_the_dock_with_destination_and_continues_while_browsing() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::copy_starts_in_the_dock_with_destination_and_continues_while_browsing",
        || {
            let fixture = Fixture::new();
            let name = format!("{}-desktop-amd64.iso", "long-name-".repeat(10));
            let id = fixture.transfer(&name, false);
            let progress = fixture.progress(id);
            assert!(crate::ui::window::visible_modal_layer(&fixture.window).is_none());
            assert!(!fixture.view.browser().has_foreground_operation());
            assert!(
                !fixture.blur.imp().blurred.get(),
                "docked progress must not blur browsing"
            );
            fixture.update(id, &name, 45);
            let card = progress_card(&progress);
            assert_eq!(card.status.text(), "45%");
            assert_eq!(card.info.text(), name);
            assert_eq!(card.count.text(), "0/1");
            assert_eq!(
                card.destination.text(),
                format!("→ {}", fixture.temp.path().display())
            );
            fixture.operations.emit(
                id,
                OperationEvent::TransferProgress {
                    request_id: id,
                    completed_items: 1,
                    completed_files: 1,
                    total_files: Some(2),
                    current_file: Some("next-file.txt".to_owned()),
                    created_location: None,
                    transferred_bytes: 50,
                    total_bytes: Some(100),
                },
            );
            assert_eq!(card.info.text(), "next-file.txt");
            assert_eq!(card.count.text(), "1/2");
            assert_eq!(card.status.text(), "50%");
            let elsewhere = fixture.temp.path().join("elsewhere");
            std::fs::create_dir(&elsewhere).expect("browsing destination");
            fixture.view.browser().navigate(Location::local(&elsewhere));
            assert_eq!(
                fixture.view.browser().active_location(),
                Some(Location::local(elsewhere))
            );
            assert!(!fixture.operations.cancelled(id));
            fixture.update(id, &name, 75);
            assert_eq!(card.status.text(), "75%");
            fixture.finish(id);
            assert!(card.root.parent().is_none());
        },
    );
}

#[test]
fn docked_progress_does_not_change_an_existing_modal_blur() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::docked_progress_does_not_change_an_existing_modal_blur",
        || {
            let fixture = Fixture::new();
            crate::ui::modal::show_error_dialog(
                &fixture.window,
                "Existing modal",
                "Keep this modal open.",
            );
            let layer =
                crate::ui::window::visible_modal_layer(&fixture.window).expect("existing modal");
            assert!(fixture.blur.imp().blurred.get());
            let id = fixture.transfer("background.txt", false);
            fixture.progress(id);
            assert!(fixture.blur.imp().blurred.get());
            assert_eq!(
                crate::ui::window::visible_modal_layer(&fixture.window),
                Some(layer.clone())
            );
            fixture.finish(id);
            assert!(fixture.blur.imp().blurred.get());
            assert_eq!(
                crate::ui::window::visible_modal_layer(&fixture.window),
                Some(layer)
            );
        },
    );
}

#[test]
fn docked_copy_archive_and_deletion_update_and_cancel_independently() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::docked_copy_archive_and_deletion_update_and_cancel_independently",
        || {
            let fixture = Fixture::new();
            let first = fixture.transfer("first.txt", false);
            let first_progress = fixture.progress(first);
            let second = fixture.transfer("second.txt", false);
            let second_progress = fixture.progress(second);
            fixture.view.browser().compress(
                vec![entry(Location::local(fixture.temp.path().join("source")))],
                Location::local(fixture.temp.path()),
                "bundle".into(),
                TransferConflict::FailIfExists,
                ArchiveFormat::Zip,
                None,
            );
            let archive = fixture
                .view
                .browser()
                .last_started_operation()
                .expect("archive started");
            let archive_progress = fixture.progress(archive);
            let deleted = entry(Location::local(fixture.temp.path().join("deleted.txt")));
            fixture.view.browser().delete(vec![deleted.clone()], false);
            let deletion = fixture
                .view
                .browser()
                .last_started_operation()
                .expect("deletion started");
            let deletion_progress = fixture.progress(deletion);
            fixture.update(first, "first.txt", 25);
            fixture.update(second, "second.txt", 65);
            fixture.operations.emit(
                archive,
                OperationEvent::ArchiveProgress {
                    request_id: archive,
                    completed: 1,
                    total: 2,
                },
            );
            assert_eq!(progress_card(&first_progress).status.text(), "25%");
            let second_card = progress_card(&second_progress);
            assert_eq!(second_card.status.text(), "65%");
            assert_eq!(progress_card(&archive_progress).status.text(), "50%");
            assert_eq!(
                progress_card(&deletion_progress).destination.text(),
                "→ Trash"
            );
            second_card.cancel.emit_clicked();
            assert!(fixture.operations.cancelled(second));
            assert!(!second_card.cancel.is_sensitive());
            assert!(!fixture.operations.cancelled(first));
            assert!(!fixture.operations.cancelled(archive));
            assert!(!fixture.operations.cancelled(deletion));
            fixture.operations.emit(
                deletion,
                OperationEvent::Deleted {
                    request_id: deletion,
                    locations: vec![deleted.location],
                },
            );
            fixture.finish(first);
            fixture.operations.emit(
                archive,
                OperationEvent::Compressed {
                    request_id: archive,
                    archive_name: "bundle.zip".into(),
                    archive: Location::local(fixture.temp.path().join("bundle.zip")),
                    original: None,
                },
            );
            assert!(
                fixture
                    .view
                    .state
                    .background_file_progress
                    .borrow()
                    .contains_key(&second)
            );
            assert!(
                !fixture
                    .view
                    .state
                    .background_file_progress
                    .borrow()
                    .contains_key(&first)
            );
            assert!(crate::ui::window::visible_modal_layer(&fixture.window).is_none());
        },
    );
}

#[test]
fn background_failure_preserves_an_exclusive_foreground_move() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::background_failure_preserves_an_exclusive_foreground_move",
        || {
            let fixture = Fixture::new();
            let first = fixture.transfer("first.txt", false);
            fixture.progress(first);
            let next = fixture.transfer("moving.txt", true);
            pump_until(|| {
                fixture
                    .view
                    .state
                    .file_progress()
                    .file_progress_view
                    .borrow()
                    .is_some()
            });
            fixture.update(next, "moving.txt", 50);
            assert!(
                fixture.blur.imp().blurred.get(),
                "foreground progress should retain modal blur"
            );
            fixture.operations.emit(
                first,
                OperationEvent::TransferFailed {
                    request_id: first,
                    message: "Test failure".into(),
                    completed_locations: Vec::new(),
                },
            );
            assert!(fixture.view.browser().is_current_operation(next));
            assert!(!fixture.operations.cancelled(next));
            assert_eq!(
                fixture
                    .view
                    .state
                    .file_progress()
                    .file_progress_view
                    .borrow()
                    .as_ref()
                    .expect("foreground move progress")
                    .transfer_percent
                    .text(),
                "50%"
            );
            fixture.finish(next);
        },
    );
}

#[test]
fn archive_activity_is_retained_before_display_and_while_a_large_member_is_running() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::archive_activity_is_retained_before_display_and_while_a_large_member_is_running",
        || {
            let fixture = Fixture::new();
            set_file_progress_delay_for_test(Duration::from_millis(60));
            fixture.view.browser().compress(
                vec![entry(Location::local(
                    fixture.temp.path().join("large.bin"),
                ))],
                Location::local(fixture.temp.path()),
                "bundle".into(),
                TransferConflict::FailIfExists,
                ArchiveFormat::Zip,
                None,
            );
            let id = fixture
                .view
                .browser()
                .last_started_operation()
                .expect("compression started");
            fixture.operations.emit(
                id,
                OperationEvent::ArchiveProgress {
                    request_id: id,
                    completed: 0,
                    total: 1,
                },
            );
            let progress = fixture.progress(id);
            let card = progress_card(&progress);
            assert_eq!(card.meta.text(), "Compressing…");
            assert_eq!(
                card.destination.text(),
                format!("→ {}", fixture.temp.path().display())
            );
            fixture.operations.emit(
                id,
                OperationEvent::ArchiveProgress {
                    request_id: id,
                    completed: 0,
                    total: 1,
                },
            );
            assert_eq!(card.meta.text(), "Compressing…");
            fixture.operations.emit(
                id,
                OperationEvent::ArchiveProgress {
                    request_id: id,
                    completed: 1,
                    total: 2,
                },
            );
            assert_eq!(card.status.text(), "50%");
            fixture.operations.emit(
                id,
                OperationEvent::Compressed {
                    request_id: id,
                    archive_name: "bundle.zip".into(),
                    archive: Location::local(fixture.temp.path().join("bundle.zip")),
                    original: None,
                },
            );
        },
    );
}
