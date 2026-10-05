// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use super::*;

#[test]
fn frame_work_runs_outside_dispatch_and_cancelled_work_never_runs() {
    crate::test_support::gtk_test(
        "ui::frame::tests::frame_work_runs_outside_dispatch_and_cancelled_work_never_runs",
        || {
            let window = gtk::Window::new();
            window.present();
            let done = Rc::new(Cell::new(false));
            let cancelled = Rc::new(Cell::new(false));
            let cancelled_for_task = cancelled.clone();
            let task = FrameTask::new(Some(window.upcast_ref()), move || {
                cancelled_for_task.set(true)
            });
            drop(task);
            let done_for_task = done.clone();
            let _task = FrameTask::new(Some(window.upcast_ref()), move || done_for_task.set(true));
            assert!(!done.get());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !done.get() {
                assert!(Instant::now() < deadline, "frame work was not dispatched");
                glib::MainContext::default().iteration(false);
            }
            assert!(!cancelled.get());
            window.destroy();
        },
    );
}

#[test]
fn unattached_producers_dispatch_on_idle_without_waiting_for_a_frame() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let done = Rc::new(Cell::new(false));
    let done_for_task = done.clone();
    let _task = FrameTask::new(None, move || done_for_task.set(true));
    assert!(!done.get());
    while context.pending() {
        context.iteration(false);
    }
    assert!(done.get());
}
