// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn transfer_progress_reports_a_byte_fraction_when_the_total_is_known() {
    assert_eq!(
        transfer_progress_status(0, 2, 750, Some(1_000)),
        ("75%".to_owned(), Some(0.75))
    );
    assert_eq!(
        transfer_progress_status(1, 2, 1_500, Some(1_000)),
        ("100%".to_owned(), Some(1.0))
    );
    assert_eq!(
        transfer_progress_status(0, 2, 1, Some(1_000)),
        ("1%".to_owned(), Some(0.001))
    );
}

#[test]
fn zero_byte_transfer_progress_tracks_completed_items() {
    assert_eq!(
        transfer_progress_status(0, 2, 0, Some(0)),
        ("0%".to_owned(), Some(0.0))
    );
    assert_eq!(
        transfer_progress_status(1, 2, 0, Some(0)),
        ("50%".to_owned(), Some(0.5))
    );
    assert_eq!(
        transfer_progress_status(2, 2, 0, Some(0)),
        ("100%".to_owned(), Some(1.0))
    );
    assert_eq!(
        transfer_progress_status(0, 0, 0, Some(0)),
        ("Preparing…".to_owned(), None)
    );
}

#[test]
fn transfer_progress_is_indeterminate_when_the_total_is_unknown() {
    assert_eq!(
        transfer_progress_status(0, 2, 0, None),
        ("Preparing…".to_owned(), None)
    );
    assert_eq!(
        transfer_progress_status(0, 2, 1_200, None),
        ("1.2 kB copied".to_owned(), None)
    );
}

#[test]
fn small_operations_delay_progress_while_large_or_unbounded_operations_show_it_immediately() {
    assert!(!should_show_progress_immediately(1));
    assert!(!should_show_progress_immediately(
        IMMEDIATE_PROGRESS_ITEM_COUNT - 1
    ));
    assert!(should_show_progress_immediately(
        IMMEDIATE_PROGRESS_ITEM_COUNT
    ));
    assert!(should_show_progress_immediately(0));
}

#[test]
fn archive_preparation_and_member_encoding_keep_activity_visible() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::archive_preparation_and_member_encoding_keep_activity_visible",
        || {
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&view.widget()));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let state = &view.state;
            state.handle(&crate::app::BrowserEvent::ArchiveStarted { total: 0 });
            assert!(state.pending_file_progress.borrow().is_none());
            let (status, activity, progress) = {
                let current = state.file_progress_view.borrow();
                let current = current.as_ref().expect("immediate archive feedback");
                (
                    current.status.clone(),
                    current.archive_activity.clone(),
                    current.progress.clone(),
                )
            };
            assert_eq!(status.text(), "Preparing…");
            assert!(activity.is_visible() && activity.is_spinning());
            for (completed, total, expected) in [
                (0, 8, "Preparing…"),
                (1, 8, "1 / 8 files"),
                (1, 8, "1 / 8 files"),
                (8, 8, "8 / 8 files"),
            ] {
                state.handle(&crate::app::BrowserEvent::ArchiveProgress { completed, total });
                assert_eq!(status.text(), expected);
                assert!(activity.is_visible() && activity.is_spinning());
                if completed > 0 {
                    assert_eq!(progress.fraction(), completed as f64 / total as f64);
                }
            }
            state.dismiss_file_operation_progress();
            assert!(!activity.is_spinning());
            assert!(state.file_progress_view.borrow().is_none());
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn progress_card_keeps_cancel_available_until_terminal_dismissal() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::progress_card_keeps_cancel_available_until_terminal_dismissal",
        || {
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&view.widget()));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let state = &view.state;
            let cancellations = Rc::new(Cell::new(0));
            for title in [
                "Working",
                "Copying items",
                "Moving items",
                "Deleting items",
                "Restoring items",
                "Emptying Trash",
            ] {
                let cancelled = cancellations.clone();
                state.show_file_operation_progress(
                    16,
                    crate::assets::icons::FILE_ARCHIVE,
                    title,
                    Rc::new(move || cancelled.set(cancelled.get() + 1)),
                );
                let (layer, status, progress) = {
                    let current = state.file_progress_view.borrow();
                    let current = current.as_ref().expect("progress view");
                    (
                        current.layer.clone(),
                        current.status.clone(),
                        current.progress.clone(),
                    )
                };
                let controllers = layer.observe_controllers();
                let before = cancellations.get();
                assert!(!layer.has_css_class("dismissing"), "{title}");
                assert!(layer.is_sensitive());
                assert_eq!(layer.parent().as_ref(), Some(view.overlay().upcast_ref()));

                state.update_archive_progress(4, 16);
                assert_eq!(status.text(), "4 / 16 files");
                assert_eq!(progress.fraction(), 0.25);
                state.update_transfer_progress(8, 50, Some(100));
                assert_eq!(status.text(), "50%");
                assert_eq!(progress.fraction(), 0.5);
                state.update_item_progress(12, 16);
                assert_eq!(status.text(), "75%");
                state.update_empty_trash_progress(12);
                assert_eq!(status.text(), "12 items deleted");

                let cancel = descendants(layer.upcast_ref())
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
                    .find(|button| button.has_css_class("job-action"))
                    .expect("Cancel button");
                assert!(cancel.is_sensitive());
                cancel.emit_clicked();
                assert_eq!(cancellations.get(), before + 1);
                let escape = (0..controllers.n_items())
                    .find_map(|index| {
                        controllers
                            .item(index)
                            .and_downcast::<gtk::EventControllerKey>()
                    })
                    .expect("Escape controller");
                let key = gtk::gdk::Key::Escape;
                assert!(escape.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
                ));
                assert_eq!(cancellations.get(), before + 2);
                assert!(
                    !layer.has_css_class("dismissing"),
                    "wait for cancellation to finish"
                );

                let dismissed = Rc::new(Cell::new(false));
                let dismissed_callback = dismissed.clone();
                state.dismiss_file_operation_progress_then(move || {
                    dismissed_callback.set(true);
                });
                assert!(state.file_progress_view.borrow().is_none());
                assert!(layer.has_css_class("dismissing"));
                assert!(!dismissed.get());
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while layer.parent().is_some() {
                    assert!(std::time::Instant::now() < deadline, "terminal dismissal");
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert!(dismissed.get());
            }
            let ordinary = crate::ui::modal::modal_layer(
                &gtk::Label::new(Some("Confirmation")),
                &overlay,
                None,
                None,
            );
            overlay.add_overlay(&ordinary);
            let controllers = ordinary.observe_controllers();
            let click = (0..controllers.n_items())
                .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
                .expect("ordinary backdrop gesture");
            click.emit_by_name::<()>("pressed", &[&1i32, &0.0f64, &0.0f64]);
            assert!(ordinary.has_css_class("dismissing"));
            window.destroy();
            view.browser().clear_observer();
        },
    );
}

#[test]
fn known_size_copy_keeps_writing_to_device_until_the_dialog_can_show_it() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::known_size_copy_keeps_writing_to_device_until_the_dialog_can_show_it",
        || {
            let view = crate::ui::browser::BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                crate::ui::browser::PeekBehavior::default(),
            );
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&view.widget()));
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            let state = &view.state;

            state.handle(&crate::app::BrowserEvent::TransferStarted {
                total: 16,
                moving: false,
            });
            state.handle(&crate::app::BrowserEvent::TransferProgress {
                completed_items: 16,
                transferred_bytes: 1_000,
                total_bytes: Some(1_000),
            });
            spin_until(|| {
                flush_dialog(state)
                    .is_some_and(|(_, indeterminate, pulsing)| !indeterminate && !pulsing)
            });
            state.handle(&crate::app::BrowserEvent::FlushingToDevice);
            assert_eq!(
                flush_dialog(state).expect("open flush"),
                ("Writing to device…".to_owned(), true, true)
            );
            state.handle(&crate::app::BrowserEvent::TransferProgress {
                completed_items: 16,
                transferred_bytes: 1_000,
                total_bytes: Some(1_000),
            });
            std::thread::sleep(std::time::Duration::from_millis(150));
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(
                flush_dialog(state).expect("flush survives a later byte update"),
                ("Writing to device…".to_owned(), true, true)
            );

            state.handle(&crate::app::BrowserEvent::TransferFinished {
                moved_locations: Vec::new(),
            });
            spin_until(|| state.file_progress_view.borrow().is_none());

            state.handle(&crate::app::BrowserEvent::TransferStarted {
                total: 1,
                moving: false,
            });
            state.handle(&crate::app::BrowserEvent::TransferProgress {
                completed_items: 1,
                transferred_bytes: 1_000,
                total_bytes: Some(1_000),
            });
            assert!(state.file_progress_view.borrow().is_none());
            state.handle(&crate::app::BrowserEvent::FlushingToDevice);
            assert!(state.file_progress_view.borrow().is_none());
            spin_until(|| state.file_progress_view.borrow().is_some());
            assert_eq!(
                flush_dialog(state).expect("delayed flush"),
                ("Writing to device…".to_owned(), true, true)
            );

            window.destroy();
            view.browser().clear_observer();
        },
    );
}

fn flush_dialog(state: &crate::ui::browser::ViewState) -> Option<(String, bool, bool)> {
    let view = state.file_progress_view.borrow();
    let view = view.as_ref()?;
    Some((
        view.status.text().to_string(),
        view.indeterminate.get(),
        view.pulse_source.borrow().is_some(),
    ))
}

fn spin_until(mut ready: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "timed out waiting for the copy dialog"
        );
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
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
