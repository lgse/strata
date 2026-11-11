//! A GLib main context on its own thread, marshalled into Rune.
//!
//! Rune lacks: a GLib main loop. Rune runs winit's event loop and never iterates a
//! `GMainContext`, so GIO's async calls, signals and file monitors never dispatch in a Rune app.
//! Volumes, mount operations, monitors, `AppInfo` and GVfs all need one.
//!
//! Rune issue: S5 drafts.
//!
//! Work package: P9, `docs/work/phase-1-leaves/P9-glib-bridge.md`.
//!
//! # The shape
//!
//! [`GlibThread::start`] runs a fresh `MainContext` (never the global default one) and its
//! `MainLoop` on a thread named [`THREAD_NAME`]. Work goes there from any thread through the
//! [`Glib`] handle; answers come back into objects the app keeps in its state, and each answer
//! calls the [`rune::Waker`] passed to `start`, so the app's `poll` drains them on Rune's loop:
//!
//! - [`Glib::spawn`] runs a GIO future and lands its output in a [`Task`].
//! - [`Glib::listen`] connects GObject signals and queues their arguments in a [`Signal`].
//!   [`Glib::channel`] and [`Glib::attach`] are its two halves, for several objects or signals
//!   feeding one queue.
//! - [`Glib::requests`] is the path for a handler that must be answered, such as
//!   `MountOperation::ask-password`: the GLib side awaits [`Asker::ask`] while the app answers
//!   the [`Request`] from `poll`.
//!
//! ```ignore
//! struct State {
//!     glib: Option<GlibThread>,
//!     size: Option<Task<Result<i64, String>>>,
//! }
//!
//! // AppSpec::setup: start the bridge with the setup waker.
//! fn setup(s: &mut State, waker: rune::Waker) {
//!     s.glib = GlibThread::start(waker).ok();
//! }
//!
//! // A handler: ask for a file's size. Only the path crosses; the File lives on the GLib thread.
//! fn measure(s: &mut State, path: std::path::PathBuf) {
//!     let Some(glib) = &s.glib else { return };
//!     s.size = Some(glib.handle().spawn(move || async move {
//!         let file = gio::File::for_path(&path);
//!         let info = file.query_info_future("standard::size", Default::default(), Default::default());
//!         info.await.map(|info| info.size()).map_err(|e| e.to_string())
//!     }));
//! }
//!
//! // AppSpec::poll: true only when the answer landed.
//! fn poll(s: &mut State, _: f64, _: &mut rune::Tx) -> bool {
//!     let Some(size) = s.size.as_ref().and_then(Task::take) else { return false };
//!     // … show `size`
//!     true
//! }
//! ```
//!
//! An app that has an `AppHandle` instead of the setup waker passes
//! `Arc::new(move || handle.wake())`. The bridge doesn't use `AppHandle::update`: answers always
//! land in state and are read in `poll`, which a headless `Runtime` runs the same way.
//!
//! # Which side touches what
//!
//! - **GObjects stay on the GLib thread.** Create, use and drop every `glib` and `gio` object
//!   inside the closures and futures the bridge runs, even the types gtk-rs marks `Send`
//!   (`gio::File`, `gio::Cancellable`). Only closures, paths, URIs and plain data cross over.
//! - **What crosses is `Send`:** the closures handed to [`Glib`], task outputs, signal messages,
//!   questions and answers. [`GlibThread`], [`Glib`], [`Task`], [`Signal`], [`Attachment`],
//!   [`Inbox`], [`Requests`], [`Request`], [`Emitter`] and [`Asker`] are `Send` (for `Send`
//!   payloads); [`Keep`] is not, and lives only on the GLib thread.
//! - **Spawn GLib-side futures with [`spawn_local`]**, not `glib::spawn_future_local`. The bridge
//!   tracks them: shutdown drops them first, and [`LocalTask::abort`] drops one early. An
//!   untracked future still pending at shutdown is dropped when the context is finalized, which
//!   the bridge does on the GLib thread as it exits.
//! - **Timers and idles on the bridge's context.** Use `glib::timeout_future`, or a source
//!   attached to `MainContext::ref_thread_default()`. Never `glib::timeout_add_local` or
//!   `idle_add_local`: they attach to the global default context, which the bridge doesn't run,
//!   so they never fire.
//! - **Async calls only.** A blocking GIO call stalls every task on the bridge, and shutdown waits
//!   for it only up to its bound before it leaves the thread running.
//! - **Keep the context on the bridge.** `MainContext` is `Send`, but don't move a clone of the
//!   bridge's context (or a GLib source or object attached to it) off the GLib thread. If such a
//!   reference were the last one, the context would be finalized on another thread, and an
//!   untracked future still pending would then panic inside a C callback and **abort the
//!   process**. That is the one abort path left; the bridge's own references all drop on the GLib
//!   thread.
//! - **Singletons bind to the first caller.** `gio::VolumeMonitor::get()` emits its signals in the
//!   thread-default context of the thread that first calls it, so make that first call on the
//!   bridge.
//!
//! # Shutdown
//!
//! Dropping the [`GlibThread`] (with the app state that owns it, so also when a headless
//! `Runtime` drops) stops the loop. On the GLib thread it then drops every tracked future and
//! every [`Keep`] (disconnecting its handlers), runs what is still queued, and exits; the drop
//! waits for that, up to one second. Tasks still pending report [`TaskState::Stopped`], and work
//! sent through a [`Glib`] handle afterwards stops at once. Last, the thread drops its context
//! and loop, finalizing what is still attached there.
//!
//! # What Strata moves onto it
//!
//! - Volumes, drives and mounts (`adapters/volume`, `gio::VolumeMonitor`).
//! - Mount operations: password, question and process prompts (`gio::MountOperation`, through
//!   [`Glib::requests`]; the app's prompts answer them).
//! - Folder monitors (`adapters/local_files/monitors.rs`, `gio::FileMonitor`).
//! - Open With and launching (`gio::AppInfo`, `gio::AppLaunchContext`).
//! - GVfs and remote locations (`adapters/gio_location`), the trash (`trash://`) and camera
//!   photos (gphoto2 through GVfs).
//! - GIO's async copies and their progress (`adapters/local_operations`).
//!
//! Strata's `glib::timeout_add_local` and `idle_add_local` calls don't move here: they become
//! Rune timers (`Tx::wake_after`) and `poll`.

mod channel;
mod registry;
mod request;
mod signal;
mod task;
mod thread;

pub use channel::{Emitter, Inbox};
pub use registry::{LocalTask, spawn_local};
pub use request::{Asker, Request, Requests};
pub use signal::{Attachment, Keep, Signal};
pub use task::{Task, TaskState};
pub use thread::{Glib, GlibThread, ShutdownError, THREAD_NAME};

#[cfg(test)]
mod tests;
