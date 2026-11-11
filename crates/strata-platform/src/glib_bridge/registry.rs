//! What the GLib thread keeps alive: the tasks it runs and the [`Keep`]s it holds, so that
//! cancelling and shutdown drop them on that thread.
//!
//! gtk-rs wraps a local future in a `ThreadGuard` that panics when it is dropped on another
//! thread, and GObjects must not be finalized off the thread that uses them. So nothing here is
//! ever dropped by the Rune side: a [`Task`](super::Task) or [`Attachment`](super::Attachment)
//! that goes away sends a closure here, which drops the entry on the GLib thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;

use super::signal::Keep;

thread_local! {
    /// `Some` only on a bridge thread, from start to exit.
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Key {
    /// A [`Glib::spawn`](super::Glib::spawn) task, by the id its `Task` holds.
    Task(u64),
    /// A [`spawn_local`] future, numbered on the GLib thread.
    Local(u64),
}

struct Registry {
    open: bool,
    next_local: u64,
    tasks: HashMap<Key, glib::JoinHandle<()>>,
    keeps: HashMap<u64, Keep>,
}

/// Makes this thread a bridge thread. Called once, by the bridge thread as it starts.
pub(super) fn open() {
    REGISTRY.with_borrow_mut(|r| {
        *r = Some(Registry {
            open: true,
            next_local: 0,
            tasks: HashMap::new(),
            keeps: HashMap::new(),
        });
    });
}

/// Whether this is a bridge thread that still takes work: false once shutdown began.
pub(super) fn is_open() -> bool {
    REGISTRY
        .try_with(|r| r.borrow().as_ref().is_some_and(|r| r.open))
        .unwrap_or(false)
}

/// Runs `future` on this thread's context and tracks it under `key`. Drops it instead, returning
/// false, when this isn't a bridge thread or the bridge is shutting down.
pub(super) fn spawn_tracked(key: Key, future: impl Future<Output = ()> + 'static) -> bool {
    if !is_open() {
        return false;
    }
    let forget = Forget(key);
    let handle = glib::MainContext::ref_thread_default().spawn_local(async move {
        let _forget = forget;
        future.await;
    });
    REGISTRY.with_borrow_mut(|r| {
        if let Some(r) = r {
            r.tasks.insert(key, handle);
        }
    });
    true
}

/// Removes a task's entry when its future is dropped: finished, panicked or aborted.
struct Forget(Key);

impl Drop for Forget {
    fn drop(&mut self) {
        let key = self.0;
        // The handle is dropped after the borrow ends. `try_with`: this may run while the
        // thread's locals are being destroyed.
        let handle = REGISTRY
            .try_with(|r| r.borrow_mut().as_mut().and_then(|r| r.tasks.remove(&key)))
            .ok()
            .flatten();
        drop(handle);
    }
}

/// Drops the task under `key` on this thread, if it is still running.
pub(super) fn abort(key: Key) {
    let handle = REGISTRY
        .try_with(|r| r.borrow_mut().as_mut().and_then(|r| r.tasks.remove(&key)))
        .ok()
        .flatten();
    if let Some(handle) = handle {
        handle.abort();
    }
}

/// Holds `keep` until [`release`] or shutdown. Drops it at once when the bridge is shutting down.
pub(super) fn hold(id: u64, keep: Keep) {
    let refused = REGISTRY.with_borrow_mut(|r| match r {
        Some(r) if r.open => r.keeps.insert(id, keep),
        _ => Some(keep),
    });
    drop(refused);
}

pub(super) fn release(id: u64) {
    let keep = REGISTRY
        .try_with(|r| r.borrow_mut().as_mut().and_then(|r| r.keeps.remove(&id)))
        .ok()
        .flatten();
    drop(keep);
}

/// Closes the registry, then drops every task and every [`Keep`] on this thread.
pub(super) fn teardown() {
    let (tasks, keeps) = REGISTRY.with_borrow_mut(|r| match r {
        Some(r) => {
            r.open = false;
            (std::mem::take(&mut r.tasks), std::mem::take(&mut r.keeps))
        }
        None => Default::default(),
    });
    // Outside the borrow: dropping a future or a GObject can reach the registry again.
    for (_, handle) in tasks {
        handle.abort();
    }
    drop(keeps);
}

/// Runs `future` on the GLib thread, tracked so that shutdown drops it there. Use it instead of
/// `glib::spawn_future_local` for futures started on the bridge, such as a signal handler that
/// awaits [`Asker::ask`](super::Asker::ask).
///
/// During shutdown the future is dropped without running.
///
/// # Panics
///
/// When called on any thread but a bridge thread ([`THREAD_NAME`](super::THREAD_NAME)).
pub fn spawn_local(future: impl Future<Output = ()> + 'static) -> LocalTask {
    let key = REGISTRY.with_borrow_mut(|r| {
        let r = r
            .as_mut()
            .expect("glib_bridge::spawn_local called off the GLib bridge thread");
        r.next_local += 1;
        Key::Local(r.next_local)
    });
    spawn_tracked(key, future);
    LocalTask {
        key,
        _glib_thread: PhantomData,
    }
}

/// A [`spawn_local`] future, on the GLib thread. Dropping the handle leaves the future running.
pub struct LocalTask {
    key: Key,
    /// Not `Send`: the registry it aborts through is the GLib thread's.
    _glib_thread: PhantomData<*const ()>,
}

impl LocalTask {
    /// Drops the future now, if it is still running.
    pub fn abort(self) {
        abort(self.key);
    }
}
