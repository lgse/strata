// SPDX-License-Identifier: GPL-3.0-or-later

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
fn backdrop_keeps_progress_and_cancel_available_until_terminal_dismissal() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::backdrop_keeps_progress_and_cancel_available_until_terminal_dismissal",
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
                    "Cancelling will not undo completed changes",
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
                let click = (0..controllers.n_items())
                    .find_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
                    .expect("backdrop gesture");
                let before = cancellations.get();
                click.emit_by_name::<()>("pressed", &[&1i32, &0.0f64, &0.0f64]);
                assert!(!layer.has_css_class("dismissing"), "{title}");
                assert!(layer.is_sensitive());
                assert_eq!(layer.parent().as_ref(), Some(overlay.upcast_ref()));
                assert_eq!(cancellations.get(), before, "backdrop must not cancel");

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

                let cancel = gtk::prelude::GtkWindowExt::focus(&window)
                    .and_downcast::<gtk::Button>()
                    .expect("Cancel has focus");
                assert_eq!(cancel.label().as_deref(), Some("Cancel"));
                assert!(cancel.is_visible() && cancel.is_sensitive());
                cancel.emit_clicked();
                assert_eq!(cancellations.get(), before + 1);
                let escape = (0..controllers.n_items())
                    .find_map(|index| {
                        controllers
                            .item(index)
                            .and_downcast::<gtk::EventControllerKey>()
                    })
                    .expect("Escape controller");
                assert!(escape.emit_by_name::<bool>(
                    "key-pressed",
                    &[
                        &gtk::gdk::Key::Escape,
                        &0u32,
                        &gtk::gdk::ModifierType::empty()
                    ],
                ));
                assert_eq!(cancellations.get(), before + 2);
                assert!(
                    !layer.has_css_class("dismissing"),
                    "wait for cancellation to finish"
                );

                state.dismiss_file_operation_progress();
                assert!(state.file_progress_view.borrow().is_none());
                assert!(layer.has_css_class("dismissing"));
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while layer.parent().is_some() {
                    assert!(std::time::Instant::now() < deadline, "terminal dismissal");
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            let ordinary =
                modal_layer(&gtk::Label::new(Some("Confirmation")), &overlay, None, None);
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
