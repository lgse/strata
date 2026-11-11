//! Reaching winit's `wl_display` from the `wl_surface` Rune hands out.
//!
//! Rune's `WindowHandle::with_raw_handle` gives the window handle only, never the display
//! handle (draft U9, not filed). libwayland 1.23 added `wl_proxy_get_display`, which recovers
//! the display from any proxy. It is looked up at run time in the libwayland-client winit
//! already loaded (`RTLD_NOLOAD`), never linked: wayland-sys's `libwayland_client_1_23`
//! feature would make winit's own load of libwayland fail on 1.22 hosts, and with it all of
//! winit's Wayland support.
#![expect(
    unsafe_code,
    reason = "dlopen, dlsym and wl_proxy_get_display through FFI"
)]

use std::ffi::c_void;
use std::ptr::NonNull;

use rune::WindowHandle;
use rune::raw_window_handle::RawWindowHandle;
use wayland_backend::client::Backend;

use super::super::error::{Error, Unsupported};

/// `struct wl_display *wl_proxy_get_display(struct wl_proxy *proxy)`.
pub(super) type GetDisplay = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

/// `wl_proxy_get_display` from the libwayland-client already in the process.
fn loaded() -> Option<GetDisplay> {
    // SAFETY: a NUL-terminated library name; RTLD_NOLOAD only returns a library that is
    // already loaded (winit loaded it), never loads one.
    let lib = unsafe {
        libc::dlopen(
            c"libwayland-client.so.0".as_ptr(),
            libc::RTLD_NOW | libc::RTLD_NOLOAD,
        )
    };
    if lib.is_null() {
        return None;
    }
    // SAFETY: `lib` is a live handle from dlopen, and the symbol name is NUL-terminated. The
    // handle is never closed: libwayland stays loaded for the process anyway.
    let sym = unsafe { libc::dlsym(lib, c"wl_proxy_get_display".as_ptr()) };
    if sym.is_null() {
        return None;
    }
    // SAFETY: the symbol is libwayland's `wl_proxy_get_display`, whose C signature is
    // `struct wl_display *(struct wl_proxy *)`: one pointer in, one out, as `GetDisplay`.
    Some(unsafe { std::mem::transmute::<*mut c_void, GetDisplay>(sym) })
}

/// The lookup, or why there is none. `lookup` is [`loaded`] outside tests.
pub(super) fn get_display_with(
    lookup: impl FnOnce() -> Option<GetDisplay>,
) -> Result<GetDisplay, Unsupported> {
    lookup().ok_or(Unsupported::LibwaylandBelow1_23)
}

/// The `wl_display` that owns `surface`.
///
/// # Safety
///
/// `surface` must be a live `wl_proxy`: winit's `wl_surface`, inside
/// `WindowHandle::with_raw_handle`.
///
/// With `as_if_old`, behaves as on libwayland < 1.23: the lookup finds nothing.
pub(super) unsafe fn display_of(
    surface: NonNull<c_void>,
    as_if_old: bool,
) -> Result<NonNull<c_void>, Unsupported> {
    let get = get_display_with(|| if as_if_old { None } else { loaded() })?;
    // SAFETY: `get` is wl_proxy_get_display and the caller guarantees `surface` is a live
    // proxy; it only reads the proxy's display field.
    let display = unsafe { get(surface.as_ptr()) };
    NonNull::new(display).ok_or(Unsupported::LibwaylandBelow1_23)
}

/// winit's display, from the window's surface. Call it on the UI thread.
pub(super) fn winit_display(
    window: &WindowHandle,
    as_if_old: bool,
) -> Result<NonNull<c_void>, Error> {
    window
        .with_raw_handle(|h| match h.as_raw() {
            RawWindowHandle::Wayland(w) => {
                // SAFETY: inside `with_raw_handle` the handle keeps winit's window, and so its
                // `wl_surface` proxy, alive; the pointer isn't kept past this call.
                let display = unsafe { display_of(w.surface, as_if_old) };
                display.map_err(Error::from)
            }
            other => Err(Error::Refused(
                if matches!(other, RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) {
                    "an X11 window has no Wayland display"
                } else {
                    "not a Linux window"
                },
            )),
        })
        .map_err(Error::WindowClosed)?
}

/// Runs `f` with a proxy for `window`'s own `wl_surface`, built on `conn` (winit's display)
/// for this call only, as Rune's `platform/activation.rs` builds one: the proxy is never kept,
/// and a request sent on it names winit's live surface. For a drag's origin (P1). Call it on
/// the UI thread. For P4's `set_parent_of` too.
#[cfg(any(feature = "dnd", feature = "foreign-parent"))]
pub(super) fn with_surface<R>(
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

#[cfg(test)]
mod tests;
