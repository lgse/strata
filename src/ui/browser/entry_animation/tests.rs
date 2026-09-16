// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::cell::Cell;

#[test]
fn animations_complete_without_frames_and_release_their_widgets() {
    crate::test_support::gtk_test(
        "ui::browser::entry_animation::tests::animations_complete_without_frames_and_release_their_widgets",
        || {
            let widget = gtk::Overlay::new();
            let weak = widget.downgrade();
            let retained = widget.clone();
            let calls = Rc::new(Cell::new(0));
            let completed = calls.clone();
            animate(
                &widget,
                Duration::from_millis(10),
                |_| panic!("unmapped widget must not receive frames"),
                move || {
                    assert!(!retained.is_mapped());
                    completed.set(completed.get() + 1);
                },
            );
            drop(widget);
            wait_until(|| calls.get() == 1);
            assert!(weak.upgrade().is_none());
        },
    );
}

#[test]
fn frame_completion_and_timeout_dispatch_only_once() {
    crate::test_support::gtk_test(
        "ui::browser::entry_animation::tests::frame_completion_and_timeout_dispatch_only_once",
        || {
            let widget = gtk::Overlay::new();
            let window = gtk::Window::builder().child(&widget).build();
            window.present();
            let calls = Rc::new(Cell::new(0));
            let completed = calls.clone();
            let frames = Rc::new(Cell::new(0));
            let rendered = frames.clone();
            animate(
                &widget,
                Duration::from_millis(10),
                move |_| rendered.set(rendered.get() + 1),
                move || completed.set(completed.get() + 1),
            );
            wait_until(|| calls.get() == 1);
            assert!(frames.get() > 0);
            let settled = Rc::new(Cell::new(false));
            let done = settled.clone();
            gtk::glib::timeout_add_local_once(Duration::from_millis(200), move || done.set(true));
            wait_until(|| settled.get());
            assert_eq!(calls.get(), 1);
            window.destroy();
        },
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "animation did not complete");
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}
