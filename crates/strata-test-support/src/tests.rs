// SPDX-License-Identifier: MIT

use std::{
    sync::{Arc, Mutex},
    thread::{self, ThreadId},
};

use gtk::{gio, glib, prelude::*};

use super::{ASYNC_MAIN_CONTEXT_DEFAULT, PrivateContext};

struct DroppedOn(Arc<Mutex<Option<ThreadId>>>);

impl Drop for DroppedOn {
    fn drop(&mut self) {
        *self.0.lock().expect("drop record") = Some(thread::current().id());
    }
}

#[test]
fn a_test_that_panics_settles_its_pending_work_on_its_own_thread() {
    type Start = fn(DroppedOn);
    let pending: [(&str, Start); 2] = [
        ("a spawned task", |marker| {
            drop(glib::MainContext::default().spawn_local(async move {
                let _marker = marker;
                std::future::pending::<()>().await;
            }));
        }),
        ("a GIO call", |marker| {
            gio::File::for_path("/").query_info_async(
                gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
                gio::Cancellable::NONE,
                move |_| drop(marker),
            );
        }),
    ];
    for (row, start) in pending {
        let dropped = Arc::new(Mutex::new(None));
        let marker = DroppedOn(dropped.clone());
        let failing = thread::spawn(move || {
            let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock();
            start(marker);
            panic!("the failing test");
        });
        let failing_thread = failing.thread().id();

        assert!(failing.join().is_err(), "{row}");
        assert_eq!(
            *dropped.lock().expect("drop record"),
            Some(failing_thread),
            "{row}"
        );
    }
}

struct SpawnsOnDrop {
    context: glib::MainContext,
    marker: Option<DroppedOn>,
}

impl Drop for SpawnsOnDrop {
    fn drop(&mut self) {
        let marker = self.marker.take();
        let context = &self.context;
        context
            .with_thread_default(|| {
                drop(context.spawn_local(async move {
                    let _marker = marker;
                    std::future::pending::<()>().await;
                }));
            })
            .expect("context");
    }
}

#[test]
fn a_task_attached_while_releasing_a_private_context_is_dropped_on_its_own_thread() {
    let dropped = Arc::new(Mutex::new(None));
    let context = PrivateContext::new();
    let spawns = SpawnsOnDrop {
        context: (*context).clone(),
        marker: Some(DroppedOn(dropped.clone())),
    };
    context
        .with_thread_default(|| {
            drop(context.spawn_local(async move {
                let _spawns = spawns;
                std::future::pending::<()>().await;
            }));
        })
        .expect("private context");
    // Like a GIO pool thread's reference, this keeps the context from being finalized here.
    let held = (*context).clone();
    drop(context);

    let dropped_on = *dropped.lock().expect("drop record");
    assert_eq!(dropped_on, Some(thread::current().id()));
    drop(held);
}

#[test]
#[expect(
    unsafe_code,
    reason = "gio-rs only makes a sendable GTask through an unsafe constructor"
)]
fn releasing_a_private_context_drops_its_leftover_local_tasks_on_their_own_thread() {
    let dropped = Arc::new(Mutex::new(None));
    let marker = DroppedOn(dropped.clone());
    let context = PrivateContext::new();
    let task = context
        .with_thread_default(|| {
            drop(context.spawn_local(async move {
                let _marker = marker;
                std::future::pending::<()>().await;
            }));
            // SAFETY: the task returns once and is never treated as a `LocalTask`.
            let task = unsafe {
                gio::Task::<bool>::new(gio::Cancellable::NONE, gio::Cancellable::NONE, |_, _| {})
            };
            // SAFETY: this is the task's only result.
            unsafe { task.clone().return_result(Ok(true)) };
            while context.iteration(false) {}
            task
        })
        .expect("private context");
    drop(context);

    let dropped_on = *dropped.lock().expect("drop record");
    assert_eq!(dropped_on, Some(thread::current().id()));
    // Like a GIO pool thread still finishing the task, this holds the context's last reference.
    thread::spawn(move || drop(task))
        .join()
        .expect("the other thread releases the context");
}
