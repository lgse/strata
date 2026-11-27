// SPDX-License-Identifier: MIT

//! Multi-MIME drag and drop shim on Wayland and X11 for Rune apps.
//!
//! Rune lacks Linux `drag_out`, arbitrary MIME types with lazy writers, and copy/move/ask/link
//! negotiation ([anomalyco/rune#171](https://github.com/anomalyco/rune/issues/171),
//! [#532](https://github.com/anomalyco/rune/issues/532)).
//!
//! Call [`shutdown`] on the UI thread before the event loop tears down the display (from
//! `App::on_display_close`, `App::on_quit`, or the last window's `on_close`).
//!
//! On Wayland, starting a drag cancels Rune's held pointer capture with `Tx::cancel_pointer` and
//! swallows the resulting cancelled `Phase::Up`.

mod api;
mod hub;
mod icon;
mod local;
pub mod negotiate;
mod registry;
mod session;
mod wayland;
mod window;
mod x11;

pub use api::{
    Actions, AskReply, DRAG_THRESHOLD, DragAction, DragItem, DragSource, DropData, DropOffer,
    DropPoint, DropTarget, Error, Hover, Status, TEXT_TYPES,
};
#[doc(hidden)]
pub use hub::TestRoute;
pub use icon::{DragIcon, DragImage};
pub use local::LOCAL_MIME;
pub use window::Dnd;

pub fn shutdown() -> Result<bool, Error> {
    hub::shutdown()
}

pub fn on_diagnostic(f: impl Fn(&str) + 'static) {
    hub::set_diagnostics(std::rc::Rc::new(f));
}

#[cfg(test)]
mod tests;
