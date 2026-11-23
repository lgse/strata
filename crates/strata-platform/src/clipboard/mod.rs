// SPDX-License-Identifier: MIT

//! Multi-MIME clipboard shim on Wayland (`ext`/`wlr` data control or `wl_data_device`) and X11.
//!
//! Rune lacks multi-MIME clipboard writes (e.g. GNOME/KDE file-manager formats beside text),
//! offered-type listing, ownership-loss notifications, X11 clipboard-manager hand-off on exit,
//! a public `Selection::Primary` API, and a native display handle / pre-teardown hook.
//!
//! Call [`shutdown`] on the UI thread before the event loop ends (from `App::on_quit` and the
//! last window's `on_close`) so X11 copies hand off to any clipboard manager and the shared
//! Wayland display substrate tears down while winit's display is still alive.

mod api;
mod book;
mod content;
pub mod files;
mod handle;
mod hub;

pub use api::{
    Change, Closed, CopyId, CopyState, Data, Error, HandOff, Limits, Lost, Offered, ReadError,
    Route, Selection, Status,
};
pub use content::{Content, KEEP_AFTER_EXIT, TEXT_TYPES};
pub use files::Files;
pub use handle::Clipboard;
#[doc(hidden)]
pub use handle::TestRoute;

pub fn shutdown() -> Result<Closed, Error> {
    hub::shutdown()
}

#[doc(hidden)]
pub fn provoke_connection_failure() -> bool {
    crate::transfer::Transfer::current().is_some_and(|t| {
        t.provoke_x11_connection_failure() || t.provoke_protocol_error_on_own_connection()
    })
}

pub fn on_diagnostic(f: impl Fn(&str) + 'static) {
    hub::set_diagnostics(std::rc::Rc::new(f));
}
