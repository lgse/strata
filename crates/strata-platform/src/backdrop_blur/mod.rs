// SPDX-License-Identifier: MIT

//! Compositor-level backdrop blur behind translucent Rune windows on Linux Wayland.
//!
//! Prefers the standard cross-compositor staging protocol `ext_background_effect_v1`
//! (KDE Plasma 6.3+, Hyprland, Wayfire, wlroots) and falls back to `org_kde_kwin_blur_manager`
//! (KDE Plasma 5 / 6.0–6.2, Wayfire) when `ext_background_effect_manager_v1` is absent.
//!
//! Rune lacks, at the pinned rev (`7a707454`):
//!
//! - **Linux Wayland backdrop blur.** `El::system_material` implements macOS
//!   `NSGlassEffectView` / `NSVisualEffectView` only and does not bind Wayland
//!   `ext_background_effect_v1` or `org_kde_kwin_blur_manager`.
//! - **The native display connection.** `WindowHandle::with_raw_handle` exposes the
//!   `wl_surface` without winit's `wl_display` (draft U9, not filed), so this shim reaches
//!   winit's display through the crate's internal `transfer` substrate (`wl_proxy_get_display`,
//!   libwayland 1.23+).
//!
//! # Graceful degradation
//!
//! On unsupported compositors (such as Sway or GNOME/Mutter), X11, libwayland < 1.23, or
//! non-Linux platforms, [`BackdropBlur`] is a silent no-op ([`Protocol::None`]).
//!
//! # Lifecycle
//!
//! Keep one [`BackdropBlur`] per window (clones share the same window state):
//!
//! 1. Call [`BackdropBlur::set_enabled`] whenever the app's backdrop-blur setting changes,
//!    before or after the native window exists. Repeated calls with the same state are zero-cost
//!    no-ops.
//! 2. Call [`BackdropBlur::attach`] from `App::on_window` on the UI thread. It binds the
//!    compositor protocol once per process and immediately applies the desired enabled state.
//! 3. Call [`BackdropBlur::detach`] from the window's `on_close` (or drop the last clone) to
//!    release the surface's blur object without touching winit's `wl_surface`.
//! 4. Call [`shutdown`] (or `clipboard::shutdown` / `dnd::shutdown` / `foreign_parent::shutdown`)
//!    on the UI thread before the event loop ends to tear down the shared display substrate while
//!    winit's display is still alive.

mod wayland;

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rune::raw_window_handle::RawWindowHandle;
use rune::{AppHandle, WindowHandle};

/// Which Wayland compositor blur protocol is active for an attached window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Protocol {
    /// No compositor blur protocol is active (unattached, non-Wayland, or unsupported compositor).
    #[default]
    None,
    /// Standard cross-compositor `ext_background_effect_v1`.
    ExtBackgroundEffect,
    /// Legacy KDE/Wayfire `org_kde_kwin_blur_manager`.
    KdeBlur,
}

#[derive(Default)]
struct HandleState {
    desired: bool,
    applied: Option<bool>,
    protocol: Protocol,
    attached: Option<WindowHandle>,
}

struct Inner {
    state: Mutex<HandleState>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let attached = lock(&self.state).attached.take();
        if let Some(window) = attached {
            wayland::detach(&window);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn is_wayland(window: &WindowHandle) -> bool {
    window
        .with_raw_handle(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
        .unwrap_or(false)
}

/// Per-window handle for toggling compositor-level Wayland backdrop blur.
///
/// Cheap to clone: all clones refer to the same window's desired and applied blur state.
#[derive(Clone)]
pub struct BackdropBlur {
    inner: Arc<Inner>,
}

impl Default for BackdropBlur {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for BackdropBlur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = lock(&self.inner.state);
        f.debug_struct("BackdropBlur")
            .field("desired", &state.desired)
            .field("applied", &state.applied)
            .field("protocol", &state.protocol)
            .field("attached", &state.attached.is_some())
            .finish()
    }
}

impl BackdropBlur {
    /// Creates an unattached handle with backdrop blur initially disabled.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(HandleState::default()),
            }),
        }
    }

    /// Attaches this handle to `window` on the UI thread (from `App::on_window`) and applies
    /// the current desired state.
    ///
    /// Silently returns [`Protocol::None`] on non-Wayland windows, unsupported compositors, or
    /// when winit's Wayland display is unavailable.
    pub fn attach<S: 'static>(&self, window: &WindowHandle, app: &AppHandle<S>) -> Protocol {
        if !is_wayland(window) {
            let mut state = lock(&self.inner.state);
            state.attached = None;
            state.applied = None;
            state.protocol = Protocol::None;
            return Protocol::None;
        }

        let protocol = wayland::attach(window, app);
        let desired = {
            let mut state = lock(&self.inner.state);
            state.attached = Some(window.clone());
            state.protocol = protocol;
            state.applied = None;
            state.desired
        };

        if protocol != Protocol::None {
            wayland::apply(window, desired);
            let mut state = lock(&self.inner.state);
            if state
                .attached
                .as_ref()
                .is_some_and(|attached| attached.key() == window.key())
                && state.desired == desired
            {
                state.applied = Some(desired);
            }
        }
        protocol
    }

    /// Sets whether compositor backdrop blur should be enabled for the attached window.
    ///
    /// When unattached or unsupported, remembers `enabled` without performing any platform call.
    /// Repeated calls with the same applied state are zero-cost no-ops.
    pub fn set_enabled(&self, enabled: bool) {
        let window = {
            let mut state = lock(&self.inner.state);
            state.desired = enabled;
            if state.protocol == Protocol::None || state.applied == Some(enabled) {
                return;
            }
            let Some(window) = state.attached.clone() else {
                return;
            };
            window
        };

        wayland::apply(&window, enabled);
        let mut state = lock(&self.inner.state);
        if state
            .attached
            .as_ref()
            .is_some_and(|attached| attached.key() == window.key())
            && state.desired == enabled
        {
            state.applied = Some(enabled);
        }
    }

    /// Releases the attached window's blur object without destroying the native `wl_surface`.
    ///
    /// Safe to call when unattached, after window close, or after [`shutdown`].
    pub fn detach(&self) {
        let window = {
            let mut state = lock(&self.inner.state);
            state.applied = None;
            state.protocol = Protocol::None;
            state.attached.take()
        };
        if let Some(window) = window {
            wayland::detach(&window);
        }
    }

    /// Whether backdrop blur is currently requested for this handle.
    pub fn is_enabled(&self) -> bool {
        lock(&self.inner.state).desired
    }

    /// Whether this handle is currently attached to a native Wayland window.
    pub fn is_attached(&self) -> bool {
        lock(&self.inner.state).attached.is_some()
    }

    /// Which compositor blur protocol is active on the attached window, if any.
    pub fn protocol(&self) -> Protocol {
        lock(&self.inner.state).protocol
    }
}

/// Tears down every blur object and the shared Wayland display substrate on the UI thread before
/// the event loop ends. True if a substrate was bound.
pub fn shutdown() -> Result<bool, ShutdownError> {
    crate::transfer::Transfer::shutdown().map_err(|e| ShutdownError(e.to_string()))
}

/// [`shutdown`] refused: called off the UI thread. The substrate stays.
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
