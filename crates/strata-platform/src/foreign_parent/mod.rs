//! Foreign parenting: a Rune window transient for another process's window, from the parent
//! handle an XDG portal passes (`wayland:<xdg-foreign handle>` or `x11:<xid>`), for a portal
//! backend's dialog such as a FileChooser.
//!
//! Rune lacks, at the pinned rev (`2a44156`):
//!
//! - **A transient-for of any kind**, and a hook that runs before every native window maps.
//!   On X11, `App::on_window` runs after winit has mapped the window unless it was created
//!   hidden, and `WindowOptions { visible: false }` holds only on a window's first creation.
//!   Draft [U11](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u11),
//!   not filed.
//! - **Modality on Wayland.** `xdg_wm_dialog_v1.get_xdg_dialog` takes winit's
//!   `xdg_toplevel`, and Rune hands out only the `wl_surface`, so a Wayland window can be
//!   parented but not marked modal (U11's amendment). X11 windows get `_NET_WM_STATE_MODAL`.
//! - **The native display connection.** The shim reaches winit's Wayland display through P0's
//!   substrate (`wl_proxy_get_display`, libwayland 1.23 or later), and opens an X connection
//!   of its own. Draft
//!   [U9](https://github.com/wmfeht/strata-components/blob/main/docs/decisions/S5-upstream.md#u9),
//!   not filed.
//!
//! Work package: P4, `docs/work/phase-2-composites/P4-foreign-parent.md`. The findings behind
//! it are `docs/decisions/S3-foreign-parent.md`.
//!
//! # The shape
//!
//! Open one window per request, hidden, with [`rune::open_window`] under a key of its own (the
//! portal's request handle is unique). From its `App::on_window`, parse the handle, [`apply`]
//! it, log the [`Outcome`], then show the window:
//!
//! ```ignore
//! let parent = Parent::parse(&request.parent_window).unwrap_or(Parent::None);
//! let app = App::new(spec).window_options(WindowOptions { visible: false, ..Default::default() });
//! let handle = app.handle();
//! let app = app.on_window(move |w| {
//!     let outcome = foreign_parent::apply(w, &handle, &parent, Options::new().modal(modal).on_lost(lost));
//!     log::debug!("chooser: {outcome}");
//!     w.show();
//! });
//! rune::open_window(WindowSpec::from_app(request_key, app));
//! // In the window's on_close: foreign_parent::release(key).
//! ```
//!
//! - **Hidden, then shown, from `on_window`.** `on_window` runs on every native creation
//!   before the first frame, so on Wayland the parent lands before the compositor maps the
//!   window (sway decides to float it at the map). On X11 winit maps a visible window inside
//!   its creation, so the window must be created hidden for the window manager to read the
//!   hints at the map. A window's first creation honours `visible: false`, a recreation
//!   doesn't, which is why each request gets a window of its own. A call that comes too late
//!   is reported ([`Outcome::Parented`]'s `late`).
//! - **Wayland recreates a window on every show** (Rune destroys it on close): call
//!   [`apply`] again from `on_window` each time. A new call replaces the window's import.
//! - **[`release`]** from the window's `on_close` drops its import and detaches the window
//!   from P0's substrate. A later [`apply`] also drops the imports of windows that have
//!   closed.
//! - **[`shutdown`]** on the UI thread before the event loop ends (before `Tx::quit` or
//!   `AppHandle::quit`, and in the last window's `on_close`), as with `dnd` and `clipboard`:
//!   it destroys the imports and tears down P0's substrate while winit's display lives. Any
//!   of the three modules' shutdowns does for all of them. Without it the imports are never
//!   destroyed: the substrate is never dropped, so nothing is destroyed on a freed display.
//!
//! # What it does
//!
//! | Window | Parent | Does | Outcome |
//! | --- | --- | --- | --- |
//! | Wayland | `wayland:` | `zxdg_importer_v2.import_toplevel`, a round trip, then `set_parent_of` on the window's own surface | Parented, or [`Reason::Destroyed`] for a handle the compositor refuses |
//! | Wayland | `x11:` | Nothing: no protocol joins them | [`Reason::Mismatch`] |
//! | X11 | `x11:` | `WM_TRANSIENT_FOR`, `_NET_WM_WINDOW_TYPE_DIALOG`, and `_NET_WM_STATE_MODAL` when modal | Parented, or [`Reason::BadXid`] (centred) for an id that is no window (gone, a pixmap) or is a root |
//! | X11 | `wayland:` or none | `_NET_WM_WINDOW_TYPE_DIALOG` only | [`Reason::Mismatch`], [`Reason::NoParent`] |
//! | Wayland | none | Nothing | [`Reason::NoParent`] |
//!
//! An X11 window is a dialog whatever the parent, as GTK's portal dialog always is: sway, for
//! one, floats it. Only a bad xid makes the shim place the window ([`Fallback::Centred`]);
//! anything else parentless is left to the compositor ([`Fallback::Compositor`]), and the
//! caller may place it (a window rule of the host's, say).
//!
//! # S3's owner decisions
//!
//! The owner ruled all three on 2026-11-10, as S3 recommended. The code for them is kept in
//! one place (`policy.rs`):
//!
//! 1. **libwayland below 1.23** (Ubuntu 24.04): unsupported. libwayland 1.23 is the minimum
//!    supported version. On an older one winit's display is out of reach, and a Wayland window
//!    still opens parentless ([`Reason::NoDisplay`]) rather than failing (S3 decision 1).
//! 2. **An `x11:` parent for a Wayland window** (Electron, Java, Wine and other Xwayland
//!    apps): parentless ([`Reason::Mismatch`]), as a GTK portal dialog is today. A helper
//!    process on Xwayland is the recorded upgrade path (S3 decision 2).
//! 3. **No activation token**: the FileChooser request carries none, so a parentless window is
//!    placed and focused by the compositor's policy, and the caller also calls Rune's tokenless
//!    `focus()` on it. The shim calls no `focus()` itself: that call is Strata's (S3
//!    decision 3). GNOME is a hand test before M6.
//!
//! # Known limits
//!
//! - **Wayland: the parent going away** reaches the app through [`Options::on_lost`]; the
//!   window stays open, parentless. On X11 nothing is reported: the window manager handles a
//!   transient whose parent is gone.
//! - **Wayland: one round trip** on the UI thread per [`apply`], to hear at once whether the
//!   compositor refused the handle. Its wait has no limit: a compositor that stops answering
//!   blocks the UI thread until it answers or the connection ends (as it stalls winit's own
//!   frames meanwhile). A socket too full to take the request is waited out, up to about 1 s
//!   on the UI thread, then the call is [`Reason::Failed`] and the connection lives on.
//! - **Wayland: [`release`] in `on_close`.** It also detaches the window from P0's substrate,
//!   which asks a window's app to dispatch winit's display's events. A window closed without
//!   it is set aside once the substrate sees its turn stay unrun, a few events later, and
//!   dropped once a few more such windows pile up.
//! - **Wayland: v2 only.** A compositor with `zxdg_importer_v1` alone is
//!   [`Reason::NoImporter`]. Of Hyprland, GNOME, KDE and sway, only sway has v1 at all, and it
//!   imports a v1-exported handle (Chromium's there) through v2.
//! - **The handle must still be exported.** GTK unexports it once the request is answered:
//!   apply it while the request is open.
//! - **sway** doesn't focus a window that maps while a full-screen window is on its
//!   workspace, even a transient one: a chooser over a full-screen parent shows, unfocused.

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

    /// Whether the window is modal to its parent: the portal's `modal` option, true by
    /// default. X11 only: Wayland has no route for it (see the module docs).
    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }

    /// Runs in the window's app (through `AppHandle::update`) when the compositor ends a
    /// Wayland parenting that took: the parent closed, or its handle was unexported. The
    /// window stays open, parentless. Never runs on X11.
    pub fn on_lost(mut self, f: fn(&mut S, &mut Tx)) -> Self {
        self.on_lost = Some(f);
        self
    }

    /// A test switch for strata-platform's own tests: the first attach of P0's substrate
    /// behaves as on libwayland < 1.23, where winit's display is out of reach.
    #[doc(hidden)]
    pub fn as_if_libwayland_below_1_23(mut self) -> Self {
        self.test.as_if_libwayland_below_1_23 = true;
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

/// Makes `window` transient for `parent`, or reports why it can't. Call it on the UI thread
/// from `App::on_window`, on a window opened hidden, and show the window afterwards (see the
/// module docs). `app` is the window's own handle.
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

/// The window keyed `window_key` is closing: drops its Wayland import, which ends its
/// parenting, and detaches it from P0's substrate (shared with `dnd` and `clipboard`), which
/// stops asking its app to dispatch. Call it on the UI thread, from the window's `on_close`,
/// and only there. True if it had an import. X11 has nothing to drop: the hints go with the
/// window.
pub fn release(window_key: &str) -> bool {
    let had = wayland::release(window_key);
    if let Some(t) = crate::transfer::Transfer::current() {
        t.detach(window_key);
    }
    had
}

/// Destroys every import and tears down P0's substrate (shared with `dnd` and `clipboard`),
/// on the UI thread before the event loop ends. True if a substrate was bound.
pub fn shutdown() -> Result<bool, ShutdownError> {
    crate::transfer::Transfer::shutdown().map_err(|e| ShutdownError(e.to_string()))
}

/// [`shutdown`] refused: called off the UI thread (the one that first applied a Wayland
/// parent). The substrate stays.
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
