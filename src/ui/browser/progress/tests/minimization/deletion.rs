// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;

fn deletion_routes() -> impl Iterator<Item = (bool, BrowserMode)> {
    [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons]
        .into_iter()
        .flat_map(|mode| [(false, mode), (true, mode)])
}

fn prepare_animation(
    fixture: &Fixture,
    permanent: bool,
) -> (crate::model::FileEntry, gtk::Widget, Option<gtk::Button>) {
    let trash = (!permanent).then(|| {
        let trash = gtk::Button::with_label("Trash");
        trash.set_halign(gtk::Align::End);
        trash.set_valign(gtk::Align::End);
        fixture
            .window
            .child()
            .and_downcast::<gtk::Overlay>()
            .expect("window overlay")
            .add_overlay(&trash);
        trash
    });
    let (deleted, source) = fixture.prepare_deletion();
    if let Some(trash) = &trash {
        fixture.view.state.pending_delete_dissolve.take();
        let flight = crate::ui::browser::fly_to_trash::prepare_fly_to_trash(
            &source,
            std::iter::once(&deleted),
            trash,
        )
        .expect("prepared trash flight");
        fixture
            .view
            .state
            .pending_file_operation_animation
            .replace(Some(flight));
    }
    (deleted, source, trash)
}

fn assert_browser_receives_pointer(fixture: &Fixture, source: &gtk::Widget) {
    let point = source
        .compute_point(
            &fixture.window,
            &gtk::graphene::Point::new(source.width() as f32 / 2.0, source.height() as f32 / 3.0),
        )
        .expect("browser pointer position");
    let target = fixture
        .window
        .pick(
            f64::from(point.x()),
            f64::from(point.y()),
            gtk::PickFlags::DEFAULT,
        )
        .expect("browser pointer target");
    assert!(
        target == *source || target.is_ancestor(source),
        "background deletion intercepts browser input with {}",
        target.type_().name(),
    );
}

fn dissolve_canvas(fixture: &Fixture) -> Option<gtk::Widget> {
    let overlay = fixture.window.child().expect("window overlay");
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.type_().name() == "StrataDissolveCanvas" {
            return Some(widget);
        }
    }
    None
}

#[test]
fn progress_discards_delete_animation_without_blocking_browsing_or_replaying_it() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::deletion::progress_discards_delete_animation_without_blocking_browsing_or_replaying_it",
        || {
            for (permanent, mode) in deletion_routes() {
                for count in [1, IMMEDIATE_PROGRESS_ITEM_COUNT] {
                    let fixture = Fixture::new();
                    fixture.view.set_view_mode(mode);
                    let (deleted, source, trash) = prepare_animation(&fixture, permanent);
                    set_file_progress_delay_for_test(Duration::from_millis(60));
                    let mut entries = vec![deleted.clone()];
                    for index in 1..count {
                        entries.push(entry(Location::local(
                            fixture.temp.path().join(format!("other-{index}.txt")),
                        )));
                    }
                    fixture.view.browser().delete(entries, permanent);
                    let deletion = fixture
                        .view
                        .browser()
                        .last_started_operation()
                        .expect("deletion started");
                    assert_browser_receives_pointer(&fixture, &source);
                    let progress = fixture.progress(deletion);
                    assert_browser_receives_pointer(&fixture, &source);
                    assert!(dissolve_canvas(&fixture).is_none());
                    assert!(!fixture.view.browser().has_foreground_operation());
                    assert!(fixture.view.browser().has_background_operations());
                    assert!(crate::ui::window::visible_modal_layer(&fixture.window).is_none());
                    let card = progress_card(&progress);
                    fixture.operations.emit(
                        deletion,
                        OperationEvent::DeleteProgress {
                            request_id: deletion,
                            completed: 1,
                            total: count,
                            deleted_locations: vec![deleted.location.clone()],
                        },
                    );
                    assert_eq!(card.status.text(), crate::i18n::percent(100 / count));
                    let elsewhere = fixture.temp.path().join("elsewhere");
                    std::fs::create_dir(&elsewhere).expect("browsing destination");
                    fixture.view.browser().navigate(Location::local(&elsewhere));
                    assert_eq!(
                        fixture.view.browser().active_location(),
                        Some(Location::local(elsewhere))
                    );
                    assert!(!fixture.operations.cancelled(deletion));
                    fixture.operations.emit(
                        deletion,
                        OperationEvent::Deleted {
                            request_id: deletion,
                            locations: vec![deleted.location],
                        },
                    );
                    assert_eq!(card.title.text(), "Deletion complete");
                    assert!(!fixture.view.browser().has_background_operations());
                    assert!(
                        fixture
                            .view
                            .state
                            .pending_delete_dissolve
                            .borrow()
                            .is_none()
                    );
                    assert!(
                        fixture
                            .view
                            .state
                            .deferred_delete_empty_depth
                            .get()
                            .is_none()
                    );
                    while glib::MainContext::default().iteration(false) {}
                    assert!(dissolve_canvas(&fixture).is_none());
                    if let Some(trash) = trash {
                        assert!(!trash.has_css_class("trash-receiving"));
                    }
                }
            }
        },
    );
}

#[test]
fn fast_delete_animates_only_its_own_success_without_showing_progress() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::deletion::fast_delete_animates_only_its_own_success_without_showing_progress",
        || {
            for (permanent, mode) in deletion_routes() {
                for outcome in ["success", "unsuccessful", "failed", "cancelled", "partial"] {
                    let fixture = Fixture::new();
                    fixture.view.set_view_mode(mode);
                    let copy = fixture.transfer("copy.txt", false);
                    fixture.progress(copy);
                    let (deleted, source, trash) = prepare_animation(&fixture, permanent);
                    set_file_progress_delay_for_test(Duration::from_secs(30));
                    fixture.view.browser().delete(vec![deleted], permanent);
                    let deletion = fixture
                        .view
                        .browser()
                        .last_started_operation()
                        .expect("deletion started");
                    let progress = fixture.progress_state(deletion);
                    assert!(progress.file_progress_view.borrow().is_none());
                    assert_browser_receives_pointer(&fixture, &source);
                    fixture.view.state.handle_background_file_operation(
                        copy,
                        &crate::app::BrowserEvent::OperationFailed {
                            message: "Unrelated copy failure".into(),
                            password_failure: None,
                        },
                    );
                    fixture
                        .view
                        .state
                        .handle(&crate::app::BrowserEvent::OperationFailed {
                            message: "Unrelated foreground failure".into(),
                            password_failure: None,
                        });
                    pump_until(|| {
                        crate::ui::window::visible_modal_layer(&fixture.window).is_some()
                    });
                    fixture.view.browser().delete(
                        vec![entry(Location::local(
                            fixture.temp.path().join("other.txt"),
                        ))],
                        false,
                    );
                    let other = fixture
                        .view
                        .browser()
                        .last_started_operation()
                        .expect("other deletion started");
                    fixture.view.state.handle_background_file_operation(
                        other,
                        &crate::app::BrowserEvent::DeletionFinished { succeeded: true },
                    );
                    if permanent {
                        assert!(
                            fixture
                                .view
                                .state
                                .pending_delete_dissolve
                                .borrow()
                                .is_some()
                        );
                    }
                    if let Some(trash) = &trash {
                        assert!(!trash.has_css_class("trash-receiving"));
                    }
                    let event = match outcome {
                        "success" => crate::app::BrowserEvent::DeletionFinished { succeeded: true },
                        "unsuccessful" => {
                            crate::app::BrowserEvent::DeletionFinished { succeeded: false }
                        }
                        "failed" => crate::app::BrowserEvent::OperationFailed {
                            message: "Delete failed".into(),
                            password_failure: None,
                        },
                        "cancelled" => crate::app::BrowserEvent::OperationCancelled {
                            completed: 0,
                            failed: 0,
                            not_attempted: 1,
                            affected_locations: Default::default(),
                        },
                        _ => crate::app::BrowserEvent::OperationCompletedWithErrors {
                            message: "Partial deletion".into(),
                            retryable_locations: Vec::new(),
                            has_non_retryable_failures: true,
                        },
                    };
                    fixture
                        .view
                        .state
                        .handle_background_file_operation(deletion, &event);
                    assert!(progress.pending_file_progress.borrow().is_none());
                    assert!(progress.file_progress_view.borrow().is_none());
                    if permanent && outcome == "success" {
                        pump_until(|| dissolve_canvas(&fixture).is_some());
                        let canvas =
                            dissolve_canvas(&fixture).expect("fast permanent-delete animation");
                        pump_until(|| canvas.parent().is_none());
                    } else {
                        assert!(dissolve_canvas(&fixture).is_none());
                    }
                    if let Some(trash) = trash {
                        assert_eq!(trash.has_css_class("trash-receiving"), outcome == "success");
                        pump_until(|| !trash.has_css_class("trash-receiving"));
                    }
                    assert!(
                        fixture
                            .view
                            .state
                            .pending_delete_dissolve
                            .borrow()
                            .is_none()
                    );
                    pump_until(|| {
                        fixture
                            .view
                            .state
                            .deferred_delete_empty_depth
                            .get()
                            .is_none()
                    });
                }
            }
        },
    );
}

#[test]
fn overlapping_fast_deletions_restore_browser_input_after_playback() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::deletion::overlapping_fast_deletions_restore_browser_input_after_playback",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let fixture = Fixture::new();
                fixture.view.set_view_mode(mode);
                let (deleted, source, _) = prepare_animation(&fixture, true);
                let second = crate::ui::browser::dissolve_delete::prepare_dissolve(
                    &source,
                    std::slice::from_ref(&deleted),
                )
                .expect("second prepared dissolve");
                set_file_progress_delay_for_test(Duration::from_secs(30));
                fixture.view.browser().delete(vec![deleted.clone()], true);
                let first_id = fixture
                    .view
                    .browser()
                    .last_started_operation()
                    .expect("first deletion started");
                fixture.view.state.handle_background_file_operation(
                    first_id,
                    &crate::app::BrowserEvent::DeletionFinished { succeeded: true },
                );
                pump_until(|| dissolve_canvas(&fixture).is_some());
                fixture.view.state.pending_delete_dissolve.replace(Some((
                    fixture
                        .view
                        .browser()
                        .active_depth()
                        .expect("active deletion column"),
                    second,
                )));
                fixture.view.browser().delete(vec![deleted], true);
                let second_id = fixture
                    .view
                    .browser()
                    .last_started_operation()
                    .expect("second deletion started");
                fixture.view.state.handle_background_file_operation(
                    second_id,
                    &crate::app::BrowserEvent::DeletionFinished { succeeded: true },
                );
                pump_until(|| dissolve_canvas(&fixture).is_none());
                assert_browser_receives_pointer(&fixture, &source);
                assert!(
                    fixture
                        .view
                        .state
                        .deferred_delete_empty_depth
                        .get()
                        .is_none()
                );
            }
        },
    );
}

#[test]
fn cancelling_docked_deletion_keeps_animations_discarded_and_reports_the_result() {
    crate::test_support::gtk_test(
        "ui::browser::progress::tests::minimization::deletion::cancelling_docked_deletion_keeps_animations_discarded_and_reports_the_result",
        || {
            for (permanent, mode) in deletion_routes() {
                let fixture = Fixture::new();
                fixture.view.set_view_mode(mode);
                let (deleted, source, trash) = prepare_animation(&fixture, permanent);
                fixture.view.browser().delete(vec![deleted], permanent);
                let deletion = fixture
                    .view
                    .browser()
                    .last_started_operation()
                    .expect("deletion started");
                let progress = fixture.progress(deletion);
                let card = progress_card(&progress);
                card.cancel.emit_clicked();
                assert!(fixture.operations.cancelled(deletion));
                assert!(!card.cancel.is_sensitive());
                assert_browser_receives_pointer(&fixture, &source);
                fixture.view.state.handle_background_file_operation(
                    deletion,
                    &crate::app::BrowserEvent::OperationCancelled {
                        completed: 0,
                        failed: 0,
                        not_attempted: 1,
                        affected_locations: Default::default(),
                    },
                );
                assert!(crate::ui::window::visible_modal_layer(&fixture.window).is_some());
                assert!(dissolve_canvas(&fixture).is_none());
                assert!(
                    fixture
                        .view
                        .state
                        .pending_delete_dissolve
                        .borrow()
                        .is_none()
                );
                assert!(progress.file_progress_view.borrow().is_none());
                if let Some(trash) = trash {
                    assert!(!trash.has_css_class("trash-receiving"));
                }
            }
        },
    );
}
