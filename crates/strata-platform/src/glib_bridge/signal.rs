// SPDX-License-Identifier: MIT

//! GObject signals: what the GLib side keeps connected, and the Rune side's handle to it.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glib::prelude::*;

use super::channel::Inbox;
use super::registry;
use super::thread::Glib;

#[must_use = "a Keep releases what it holds when it drops"]
#[derive(Default)]
pub struct Keep {
    handlers: Vec<(glib::Object, glib::SignalHandlerId)>,
    // Only held: released when the Keep drops.
    objects: Vec<glib::Object>,
    values: Vec<Box<dyn Any>>,
}

impl Keep {
    pub fn new() -> Self {
        Self::default()
    }

    /// Holds a reference to `object`.
    pub fn object(mut self, object: &impl IsA<glib::Object>) -> Self {
        self.objects.push(object.clone().upcast());
        self
    }

    /// Disconnects `handler` from `object` on release. The object is held until then.
    pub fn handler(
        mut self,
        object: &impl IsA<glib::Object>,
        handler: glib::SignalHandlerId,
    ) -> Self {
        self.handlers.push((object.clone().upcast(), handler));
        self
    }

    /// Holds any other value (a `Cancellable`, an `Rc`, a guard) until release.
    pub fn value(mut self, value: impl Any) -> Self {
        self.values.push(Box::new(value));
        self
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        for (object, handler) in self.handlers.drain(..) {
            object.disconnect(handler);
        }
    }
}

#[must_use = "dropping an Attachment releases what it attached"]
pub struct Attachment {
    id: u64,
    released: Arc<AtomicBool>,
    glib: Glib,
}

impl Attachment {
    pub(super) fn new(glib: &Glib, make: impl FnOnce() -> Keep + Send + 'static) -> Self {
        let id = glib.next_id();
        let released = Arc::new(AtomicBool::new(false));
        let gone = released.clone();
        glib.run(move || {
            // Released before it was made, or drained after teardown: never make it.
            if !gone.load(Ordering::Acquire) && registry::is_open() {
                registry::hold(id, make());
            }
        });
        Attachment {
            id,
            released,
            glib: glib.clone(),
        }
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.released.store(true, Ordering::Release);
        let id = self.id;
        self.glib.run(move || registry::release(id));
    }
}

pub struct Signal<M> {
    _attachment: Attachment,
    inbox: Inbox<M>,
}

impl<M> Signal<M> {
    pub(super) fn new(attachment: Attachment, inbox: Inbox<M>) -> Self {
        Signal {
            _attachment: attachment,
            inbox,
        }
    }

    /// Takes every message queued so far, oldest first.
    pub fn drain(&self) -> std::vec::IntoIter<M> {
        self.inbox.drain()
    }

    /// Whether no message is waiting.
    pub fn is_empty(&self) -> bool {
        self.inbox.is_empty()
    }
}
