// SPDX-License-Identifier: MIT

use std::{
    sync::{Arc, Mutex},
    thread::{self, ThreadId},
};

use gtk::{gio, glib, prelude::*};

use super::ASYNC_MAIN_CONTEXT_DEFAULT;

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
