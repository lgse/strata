// SPDX-License-Identifier: MIT

#![expect(
    unsafe_code,
    reason = "wayland_backend::Backend::from_foreign_display and ObjectId::from_ptr"
)]

use std::ffi::c_void;
use std::ptr::NonNull;

use rune::WindowHandle;
use rune::raw_window_handle::RawDisplayHandle;
#[cfg(any(feature = "dnd", feature = "foreign-parent", feature = "backdrop-blur"))]
use rune::raw_window_handle::RawWindowHandle;
use wayland_backend::client::Backend;

use super::super::error::{Error, Unsupported};

/// winit's `wl_display`, from the window's display handle. Call it on the UI thread.
///
/// With `as_if_no_shared_display`, behaves as if winit's display were unreachable (for fallback tests).
pub(super) fn winit_display(
    window: &WindowHandle,
    as_if_no_shared_display: bool,
) -> Result<NonNull<c_void>, Error> {
    if as_if_no_shared_display {
        return Err(Error::from(Unsupported::NoSharedDisplay));
    }
    window
        .with_raw_display_handle(|h| match h.as_raw() {
            RawDisplayHandle::Wayland(d) => Ok(d.display),
            other => Err(Error::Refused(
                if matches!(other, RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)) {
                    "an X11 window has no Wayland display"
                } else {
                    "not a Linux window"
                },
            )),
        })
        .map_err(Error::WindowClosed)?
}

#[cfg(any(feature = "dnd", feature = "foreign-parent", feature = "backdrop-blur"))]
pub(crate) fn with_surface<R>(
    conn: &wayland_client::Connection,
    window: &WindowHandle,
    f: impl FnOnce(&wayland_client::protocol::wl_surface::WlSurface) -> R,
) -> Result<R, Error> {
    use wayland_backend::client::ObjectId;
    use wayland_client::Proxy;
    use wayland_client::protocol::wl_surface::WlSurface;
    window
        .with_raw_handle(|h| match h.as_raw() {
            RawWindowHandle::Wayland(w) => {
                // SAFETY: inside `with_raw_handle` the handle keeps winit's window, and so its
                // `wl_surface` proxy, alive; the id and proxy built from it don't outlive `f`.
                let id = unsafe {
                    ObjectId::from_ptr(WlSurface::interface(), w.surface.as_ptr().cast())
                }
                .map_err(|_| Error::Refused("the window's surface isn't a wl_surface"))?;
                let surface = WlSurface::from_id(conn, id)
                    .map_err(|_| Error::Refused("the window's surface is gone"))?;
                Ok(f(&surface))
            }
            _ => Err(Error::Refused("not a Wayland window")),
        })
        .map_err(Error::WindowClosed)?
}

/// A wayland-backend `Backend` on winit's display.
pub(super) fn foreign_backend(display: NonNull<c_void>) -> Backend {
    // SAFETY: `display` is winit's live `wl_display` (from `winit_display`). winit keeps it
    // open for the event loop, and the substrate's teardown, before `run` returns, destroys
    // every object made on it and drops this backend first. A foreign backend never
    // disconnects the display.
    unsafe { Backend::from_foreign_display(display.as_ptr().cast()) }
}
