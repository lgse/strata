// SPDX-License-Identifier: MIT

use super::*;
use gtk::glib;
use std::{cell::Cell, time::Instant};

struct Fixture {
    window: gtk::Window,
    overlay: gtk::Overlay,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self::unpresented();
        fixture.window.present();
        fixture
            .overlay
            .child()
            .expect("browser button")
            .grab_focus();
        fixture
    }

    fn unpresented() -> Self {
        let browser = gtk::Button::with_label("Browser");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&browser));
        let window = gtk::Window::builder()
            .child(&overlay)
            .default_width(640)
            .default_height(480)
            .build();
        Self { window, overlay }
    }

    fn card(&self) -> CompactProgress {
        CompactProgress::new(&self.overlay, assets::icons::COPY)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.window.destroy();
    }
}

fn pump_for(duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_removed(card: &CompactProgress) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while card.root.parent().is_some() && Instant::now() < deadline {
        pump_for(Duration::from_millis(10));
    }
    assert!(
        card.root.parent().is_none(),
        "completed notification should close"
    );
}

fn hover(card: &CompactProgress, entered: bool) {
    let controllers = card.root.observe_controllers();
    let motion = (0..controllers.n_items())
        .find_map(|index| {
            controllers
                .item(index)?
                .downcast::<gtk::EventControllerMotion>()
                .ok()
        })
        .expect("hover controller");
    if entered {
        motion.emit_by_name::<()>("enter", &[&0.0_f64, &0.0_f64]);
    } else {
        motion.emit_by_name::<()>("leave", &[]);
    }
}

#[test]
fn completion_actions_dismiss_only_their_card_without_cancelling_jobs() {
    crate::test_support::gtk_test(
        "ui::progress_dock::tests::completion_actions_dismiss_only_their_card_without_cancelling_jobs",
        || {
            let fixture = Fixture::new();
            let first = fixture.card();
            let second = fixture.card();
            let cancels = Rc::new(Cell::new(0));
            let invoked = cancels.clone();
            first.set_cancel_action(Rc::new(move || invoked.set(invoked.get() + 1)));
            first.complete.emit_clicked();
            pump_for(Duration::from_millis(120));
            assert!(
                first.root.parent().is_some(),
                "active notifications cannot be dismissed as completed"
            );
            first.cancel.emit_clicked();
            assert_eq!(cancels.get(), 1);
            first.completed("Copy complete");
            assert_eq!(first.status.text(), "100%");
            first.complete.emit_clicked();
            assert!(first.root.parent().is_none());
            assert!(second.root.parent().is_some());
            assert_eq!(cancels.get(), 1);
            second.completed("Compression complete");
            second.cancel.emit_clicked();
            assert!(second.root.parent().is_none());
        },
    );
}

#[test]
fn completed_notifications_count_down_and_auto_dismiss() {
    crate::test_support::gtk_test(
        "ui::progress_dock::tests::completed_notifications_count_down_and_auto_dismiss",
        || {
            let fixture = Fixture::unpresented();
            let card = fixture.card();
            card.complete_after("Deletion complete", Duration::from_millis(80));
            pump_for(Duration::from_millis(120));
            assert!(
                card.root.parent().is_some(),
                "expiry waits until the notification is realized"
            );
            assert_eq!(card.progress.fraction(), 1.0);
            assert_eq!(card.status.text(), "100%");
            fixture.window.present();
            fixture
                .overlay
                .child()
                .expect("browser button")
                .grab_focus();
            pump_for(Duration::from_millis(10));
            hover(&card, false);
            wait_removed(&card);
        },
    );
}

#[test]
fn hovering_pauses_and_leaving_resumes_the_remaining_countdown() {
    crate::test_support::gtk_test(
        "ui::progress_dock::tests::hovering_pauses_and_leaving_resumes_the_remaining_countdown",
        || {
            let fixture = Fixture::new();
            let card = fixture.card();
            card.complete_after("Copy complete", Duration::from_millis(300));
            pump_for(Duration::from_millis(80));
            hover(&card, true);
            let remaining = card.progress.fraction();
            assert!(remaining < 1.0);
            pump_for(Duration::from_millis(450));
            assert!(card.root.parent().is_some());
            assert_eq!(card.progress.fraction(), remaining);
            assert_eq!(card.meta.text(), " · paused");
            hover(&card, false);
            assert_eq!(
                card.progress.fraction(),
                remaining,
                "leaving must not restart the full timeout"
            );
            wait_removed(&card);
        },
    );
}

#[test]
fn already_hovered_notifications_start_paused_and_pinning_is_independent() {
    crate::test_support::gtk_test(
        "ui::progress_dock::tests::already_hovered_notifications_start_paused_and_pinning_is_independent",
        || {
            let fixture = Fixture::new();
            let first = fixture.card();
            hover(&first, true);
            first.complete_after("Copy complete", Duration::from_millis(100));
            first.pin.set_active(true);
            hover(&first, false);
            let second = fixture.card();
            second.complete_after("Format complete", Duration::from_millis(100));
            wait_removed(&second);
            assert!(first.root.parent().is_some());
            assert_eq!(first.meta.text(), " · pinned");
            first.pin.set_active(false);
            wait_removed(&first);
        },
    );
}

#[test]
fn keyboard_focus_pauses_expiry_and_window_teardown_stops_updates() {
    crate::test_support::gtk_test(
        "ui::progress_dock::tests::keyboard_focus_pauses_expiry_and_window_teardown_stops_updates",
        || {
            let fixture = Fixture::new();
            let card = fixture.card();
            card.complete_after("Copy complete", Duration::from_millis(100));
            card.complete.grab_focus();
            pump_for(Duration::from_millis(200));
            assert!(card.root.parent().is_some());
            assert_eq!(card.meta.text(), " · paused");
            fixture.window.destroy();
            let remaining = card.progress.fraction();
            pump_for(Duration::from_millis(200));
            assert_eq!(card.progress.fraction(), remaining);
            let weak = Rc::downgrade(&card.completion);
            drop(card);
            drop(fixture);
            assert!(
                weak.upgrade().is_none(),
                "notification callbacks must not retain destroyed widgets"
            );
        },
    );
}
