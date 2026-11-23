// SPDX-License-Identifier: MIT

//! Dedicated-thread GLib main context marshalled into Rune.
//!
//! Rune runs winit's event loop and never iterates a `GMainContext` (outside Rune's scope),
//! so GIO async calls, signals, and file monitors need a GLib loop of their own.
//!
//! Invariant: create, use, and drop all GLib/GIO objects and references to the bridge's
//! `MainContext` strictly on the bridge thread. Finalizing the context on another thread with
//! an untracked pending future panics inside a C callback and aborts the process.

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
