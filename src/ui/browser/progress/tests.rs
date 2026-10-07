// SPDX-License-Identifier: MIT

use super::{transfer_progress_status, transfer_rate_status};

mod minimization;

#[test]
fn transfer_status_tracks_completed_files_and_live_bytes() {
    let (percent, bytes, items, fraction) = transfer_progress_status(
        1,
        3,
        4,
        Some(9),
        1_500,
        Some(4_000),
        Some("archive-2026-09.tar"),
    );
    assert_eq!(percent, "37%");
    assert_eq!(bytes, "1.5 kB / 4 kB");
    assert_eq!(items, "4 of 9 files · archive-2026-09.tar");
    assert_eq!(fraction, Some(0.375));

    let (percent, bytes, items, fraction) =
        transfer_progress_status(2, 3, 5, Some(9), 2_000, None, None);
    assert_eq!(percent, "Transferring…");
    assert_eq!(bytes, "2 kB");
    assert_eq!(items, "5 of 9 files");
    assert_eq!(fraction, None);
    assert_eq!(
        transfer_rate_status(Some(1_000.0), 2_000, Some(12_000)),
        "1 kB/s · 10s left"
    );
    assert_eq!(transfer_rate_status(Some(1_000.0), 0, None), "1 kB/s");
    assert_eq!(
        transfer_rate_status(None, 0, Some(12_000)),
        "Calculating speed…"
    );
}

#[test]
fn live_transfer_updates_the_visible_dialog_and_device_flush() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::live_transfer_updates_the_visible_dialog_and_device_flush",
        || {
            use gtk::prelude::*;
            use std::rc::Rc;

            crate::ui::prepare_portal_ui();
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let progress_state = view.state.file_progress();
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            view.state.show_file_operation_progress(
                16,
                crate::assets::icons::COPY,
                "Copying items",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );
            progress_state
                .transfer_current_file
                .replace(Some("archive.tar".into()));
            progress_state
                .transfer_rate_bytes_per_second
                .set(Some(2_000.0));
            view.state
                .update_transfer_progress(3, 5, Some(21), 2_000, Some(10_000));
            {
                let progress = progress_state.file_progress_view.borrow();
                let progress = progress.as_ref().expect("visible progress dialog");
                assert_eq!(progress.transfer_percent.text(), "20%");
                assert_eq!(progress.transfer_bytes.text(), "2 kB / 10 kB");
                assert_eq!(
                    progress.transfer_items.text(),
                    "5 of 21 files · archive.tar"
                );
                assert_eq!(progress.transfer_rate.text(), "2 kB/s · 4s left");
                assert!(progress.transfer_header.is_visible());
                assert!(progress.transfer_footer.is_visible());
            }
            view.state.show_device_flush_status();
            assert_eq!(
                progress_state
                    .file_progress_view
                    .borrow()
                    .as_ref()
                    .expect("progress dialog while flushing")
                    .transfer_rate
                    .text(),
                "Writing to device…"
            );
            view.state.dismiss_file_operation_progress();
            window.close();
        },
    );
}

#[test]
fn completion_callbacks_wait_for_the_progress_modal_to_leave() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::completion_callbacks_wait_for_the_progress_modal_to_leave",
        || {
            use gtk::prelude::*;
            use std::{cell::RefCell, rc::Rc, time::Duration};

            crate::ui::prepare_portal_ui();
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let window = gtk::Window::builder().child(&view.overlay()).build();
            window.present();
            view.state.show_file_operation_progress(
                16,
                crate::assets::icons::COPY,
                "Copying items",
                "Cancelling will not undo completed changes",
                Rc::new(|| {}),
            );
            let layer = view
                .state
                .file_progress()
                .file_progress_view
                .borrow()
                .as_ref()
                .expect("visible progress dialog")
                .layer
                .borrow()
                .as_ref()
                .expect("modal progress layer")
                .clone();
            let completions = Rc::new(RefCell::new(Vec::new()));
            for index in 0..2 {
                let completions = completions.clone();
                let layer = layer.clone();
                view.state.dismiss_file_operation_progress_then(move || {
                    completions
                        .borrow_mut()
                        .push((index, layer.parent().is_none()));
                });
            }
            let context = glib::MainContext::default();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while completions.borrow().len() < 2 && std::time::Instant::now() < deadline {
                while context.iteration(false) {}
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(completions.borrow().as_slice(), [(0, true), (1, true)]);
            window.close();
        },
    );
}

#[test]
fn stalled_cancellation_keeps_the_dock_cancellable_only_once() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::stalled_cancellation_keeps_the_dock_cancellable_only_once",
        || {
            use gtk::prelude::*;
            use std::{cell::Cell, rc::Rc};

            crate::ui::prepare_portal_ui();
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let window = gtk::Window::builder().child(&view.overlay()).build();
            window.present();
            let progress_state = view.state.file_progress();
            let cancellations = Rc::new(Cell::new(0));
            let count = cancellations.clone();
            let on_cancel: Rc<dyn Fn()> = Rc::new(move || count.set(count.get() + 1));
            progress_state.dock_only.set(true);
            progress_state.show_file_operation_progress(
                16,
                crate::assets::icons::COPY,
                "Copying items",
                "Cancelling will not undo completed changes",
                on_cancel.clone(),
            );
            view.state
                .update_transfer_progress(0, 0, Some(4), 100, Some(1_000));
            progress_state.request_transfer_cancel(&on_cancel);
            progress_state.request_transfer_cancel(&on_cancel);
            assert_eq!(cancellations.get(), 1);
            {
                let progress = progress_state.file_progress_view.borrow();
                let progress = progress.as_ref().expect("visible transfer");
                assert_eq!(progress.title.text(), "Cancelling transfer…");
                assert!(!progress.cancel.is_sensitive());
            }
            progress_state.transfer_cancel_timed_out.set(true);
            progress_state.apply_transfer_cancel_status();
            {
                let progress = progress_state.file_progress_view.borrow();
                let progress = progress.as_ref().expect("stalled transfer");
                assert_eq!(progress.title.text(), "Device not responding");
                assert!(progress.subtitle.text().contains("do not unplug"));
                assert!(!progress.cancel.is_sensitive());
                let card = progress.compact.as_ref().expect("docked stalled transfer");
                assert!(!card.cancel.is_sensitive());
                assert!(card.info.text().contains("Do not unplug"));
                card.cancel.emit_clicked();
            }
            assert_eq!(cancellations.get(), 1);
            assert!(progress_state.file_progress_view.borrow().is_some());
            assert!(progress_state.transfer_cancel_requested.get());
            assert!(crate::ui::window::visible_modal_layer(&window).is_none());
            view.state.dismiss_file_operation_progress();
            assert!(!progress_state.transfer_cancel_requested.get());
            window.close();
        },
    );
}

#[test]
fn transfer_status_handles_empty_files_and_unknown_totals() {
    let (percent, bytes, items, fraction) =
        transfer_progress_status(1, 2, 0, Some(0), 0, Some(0), None);
    assert_eq!(percent, "50%");
    assert_eq!(bytes, "0 B / 0 B");
    assert_eq!(items, "1 of 2 items");
    assert_eq!(fraction, Some(0.5));

    let (_, bytes, items, _) = transfer_progress_status(0, 1, 0, None, 0, None, None);
    assert_eq!(bytes, "0 B");
    assert_eq!(items, "0 of 1 item");
}
