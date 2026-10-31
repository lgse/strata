// SPDX-License-Identifier: MIT

use std::{cell::RefCell, ops::Deref, time::Duration};

use gtk::{
    gio, glib,
    glib::translate::{IntoGlib, ToGlibPtr},
    prelude::*,
};

const FIRST_RECHECK: Duration = Duration::from_millis(50);
const LAST_RECHECK: Duration = Duration::from_secs(1);

thread_local! {
    static WAITING: RefCell<Vec<Box<dyn Closable>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct Closing<T: Closable>(Option<T>);

impl<T: Closable> Closing<T> {
    pub(crate) fn new(object: T) -> Self {
        Self(Some(object))
    }
}

impl<T: Closable> Deref for Closing<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref().expect("held until dropped")
    }
}

impl<T: Closable> Drop for Closing<T> {
    fn drop(&mut self) {
        if let Some(object) = self.0.take() {
            close_soon(object);
        }
    }
}

pub(crate) struct CancelOnDrop(pub(crate) gio::Cancellable);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) async fn enumerate_children(
    folder: &gio::File,
    attributes: &'static str,
    flags: gio::FileQueryInfoFlags,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Closing<gio::FileEnumerator>, glib::Error> {
    let native = folder.is_native();
    let cancellable = cancellable.cloned();
    let interrupt = cancellable.clone();
    let enumerator = gio::GioFuture::new(folder, move |folder, dropped, result| {
        let interrupt = (!native).then(|| interrupt.as_ref().unwrap_or(dropped));
        folder.enumerate_children_async(
            attributes,
            flags,
            glib::Priority::DEFAULT,
            interrupt,
            move |output| result.resolve(output.map(Closing::new)),
        );
    })
    .await?;
    if let Some(cancellable) = cancellable {
        cancellable.set_error_if_cancelled()?;
    }
    Ok(enumerator)
}

fn close_soon(object: impl Closable) {
    if object.closed() || !crate::util::ui_thread::is_current() {
        return;
    }
    if !object.pending() {
        object.close_in_background();
        return;
    }
    let first = WAITING.with_borrow_mut(|waiting| {
        waiting.push(Box::new(object));
        waiting.len() == 1
    });
    if first {
        recheck_after(FIRST_RECHECK);
    }
}

fn recheck_after(delay: Duration) {
    glib::timeout_add_once(delay, move || {
        let (ready, waiting) = WAITING.with_borrow_mut(|waiting| {
            let (ready, pending) = std::mem::take(waiting)
                .into_iter()
                .partition::<Vec<_>, _>(|object| !object.pending());
            *waiting = pending;
            (ready, !waiting.is_empty())
        });
        for object in ready.into_iter().filter(|object| !object.closed()) {
            object.close_in_background();
        }
        if waiting {
            recheck_after((delay * 2).min(LAST_RECHECK));
        }
    });
}

pub(crate) trait Closable: 'static {
    fn closed(&self) -> bool;
    fn pending(&self) -> bool;
    fn close_in_background(&self);
}

impl Closable for gio::FileEnumerator {
    fn closed(&self) -> bool {
        self.is_closed()
    }

    fn pending(&self) -> bool {
        self.has_pending()
    }

    #[expect(
        unsafe_code,
        reason = "gio-rs binds the close to a callback that only the thread starting it may run"
    )]
    fn close_in_background(&self) {
        // SAFETY: the enumerator is alive for the call and GLib keeps its own reference until
        // the close ends; GIO annotates the cancellable, callback and user data as nullable.
        unsafe {
            gio::ffi::g_file_enumerator_close_async(
                self.to_glib_none().0,
                glib::Priority::DEFAULT.into_glib(),
                std::ptr::null_mut(),
                None,
                std::ptr::null_mut(),
            );
        }
    }
}

impl Closable for gio::InputStream {
    fn closed(&self) -> bool {
        self.is_closed()
    }

    fn pending(&self) -> bool {
        self.has_pending()
    }

    #[expect(
        unsafe_code,
        reason = "gio-rs binds the close to a callback that only the thread starting it may run"
    )]
    fn close_in_background(&self) {
        // SAFETY: the stream is alive for the call and GLib keeps its own reference until the
        // close ends; GIO annotates the cancellable, callback and user data as nullable.
        unsafe {
            gio::ffi::g_input_stream_close_async(
                self.to_glib_none().0,
                glib::Priority::DEFAULT.into_glib(),
                std::ptr::null_mut(),
                None,
                std::ptr::null_mut(),
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
