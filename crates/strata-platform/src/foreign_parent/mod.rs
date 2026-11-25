// SPDX-License-Identifier: MIT

//! Foreign parenting shim for XDG portal dialogs (`wayland:<xdg-foreign>` / `x11:<xid>`).
//!
//! Rune lacks transient-for parenting, a pre-map native window hook (on X11, windows must be
//! created hidden and shown from `on_window`), and `xdg_toplevel` access for Wayland modality.
//!
//! Call [`release`] from `on_close` and [`shutdown`] on the UI thread before the event loop
//! ends (or from `App::on_display_close`) so `zxdg_imported_v2` handles tear down while winit's
//! display is still alive.

mod outcome;
mod parent;
mod policy;
mod wayland;
mod x11;

use std::fmt;

use rune::raw_window_handle::RawWindowHandle;
use rune::{AppHandle, Tx, WindowHandle};

pub use outcome::{Fallback, Outcome, Reason, WindowSystem};
pub use parent::{MAX_WAYLAND_HANDLE, Parent, ParseError, WaylandHandle, Xid};

/// How [`apply`] parents a window.
pub struct Options<S> {
    modal: bool,
    on_lost: Option<fn(&mut S, &mut Tx)>,
    test: wayland::Test,
}

impl<S> Options<S> {
    /// Modal, with no `on_lost`.
    pub fn new() -> Self {
        Options {
            modal: true,
            on_lost: None,
            test: wayland::Test::default(),
        }
    }

    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }

    pub fn on_lost(mut self, f: fn(&mut S, &mut Tx)) -> Self {
        self.on_lost = Some(f);
        self
    }

    #[doc(hidden)]
    pub fn as_if_no_shared_display(mut self) -> Self {
        self.test.as_if_no_shared_display = true;
        self
    }
}

impl<S> Default for Options<S> {
    fn default() -> Self {
        Options::new()
    }
}

impl<S> Clone for Options<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Options<S> {}

impl<S> fmt::Debug for Options<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("modal", &self.modal)
            .field("on_lost", &self.on_lost.is_some())
            .finish()
    }
}

pub fn apply<S: 'static>(
    window: &WindowHandle,
    app: &AppHandle<S>,
    parent: &Parent,
    options: Options<S>,
) -> Outcome {
    let native = window.with_raw_handle(|h| match h.as_raw() {
        RawWindowHandle::Wayland(_) => Ok(None),
        RawWindowHandle::Xlib(x) => Ok(u32::try_from(x.window).ok()),
        RawWindowHandle::Xcb(x) => Ok(Some(x.window.get())),
        other => Err(format!("{other:?}")),
    });
    let native = match native {
        Err(_) => return Outcome::parentless(Reason::WindowClosed, Fallback::Compositor),
        Ok(Err(kind)) => return Outcome::parentless(Reason::NotLinux(kind), Fallback::Compositor),
        Ok(Ok(native)) => native,
    };
    if let Some(xid) = native {
        return x11::apply(window, xid, parent, options.modal);
    }
    match parent {
        Parent::Wayland(handle) => {
            let lost = options.on_lost.map(|f| {
                let app = app.clone();
                Box::new(move || app.update(f)) as wayland::Lost
            });
            wayland::apply(window, app, handle, lost, options.test)
        }
        Parent::X11(_) => {
            wayland::release(window.key());
            policy::x11_parent_on_wayland()
        }
        Parent::None => {
            wayland::release(window.key());
            Outcome::parentless(Reason::NoParent, Fallback::Compositor)
        }
    }
}

pub fn release(window_key: &str) -> bool {
    let had = wayland::release(window_key);
    if let Some(t) = crate::transfer::Transfer::current() {
        t.detach(window_key);
    }
    had
}

pub fn shutdown() -> Result<bool, ShutdownError> {
    crate::transfer::Transfer::shutdown().map_err(|e| ShutdownError(e.to_string()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownError(String);

impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ShutdownError {}

#[cfg(test)]
mod tests;
