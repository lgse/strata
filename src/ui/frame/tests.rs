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

fn settled_count(count: &Cell<u32>) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let before = count.get();
        let quiet = Instant::now() + Duration::from_millis(300);
        while Instant::now() < quiet {
            glib::MainContext::default().iteration(false);
        }
        if count.get() == before {
            return before;
        }
        assert!(Instant::now() < deadline, "frame work never went idle");
    }
}

fn wait_for_more(count: &Cell<u32>, than: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while count.get() <= than {
        assert!(Instant::now() < deadline, "frame work did not resume");
        glib::MainContext::default().iteration(false);
    }
}

#[test]
fn active_ticks_stop_when_idle_and_resume_after_layout() {
    crate::test_support::gtk_test(
        "ui::frame::tests::active_ticks_stop_when_idle_and_resume_after_layout",
        || {
            let window = gtk::Window::new();
            let child = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            window.set_child(Some(&child));
            let count = Rc::new(Cell::new(0_u32));
            let counter = count.clone();
            tick_while_active(&child, 3, move |_| counter.set(counter.get() + 1));
            window.present();
            let idle = settled_count(&count);
            assert!(idle > 0);
            child.queue_resize();
            wait_for_more(&count, idle);
            window.destroy();
        },
    );
}

#[test]
fn layout_work_follows_a_widget_into_another_window() {
    crate::test_support::gtk_test(
        "ui::frame::tests::layout_work_follows_a_widget_into_another_window",
        || {
            let first = gtk::Window::new();
            let second = gtk::Window::new();
            let child = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            first.set_child(Some(&child));
            let count = Rc::new(Cell::new(0_u32));
            let counter = count.clone();
            after_layout(&child, move |_| counter.set(counter.get() + 1));
            first.present();
            wait_for_more(&count, 0);
            first.set_child(None::<&gtk::Widget>);
            second.set_child(Some(&child));
            second.present();
            let settled = settled_count(&count);
            first.queue_resize();
            assert_eq!(settled_count(&count), settled);
            child.queue_resize();
            wait_for_more(&count, settled);
            first.destroy();
            second.destroy();
        },
    );
}
