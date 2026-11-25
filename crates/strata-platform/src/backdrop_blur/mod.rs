// SPDX-License-Identifier: MIT

//! Compositor-level backdrop blur behind translucent Rune windows on Linux Wayland.
//!
//! Prefers `ext_background_effect_v1` and falls back to `org_kde_kwin_blur_manager`.
//! On unsupported compositors or X11, [`BackdropBlur`] is a no-op.
//!
//! Rune lacks Linux Wayland backdrop blur (`El::system_material` is macOS-only), so this shim
//! binds through the internal `transfer` substrate using `WindowHandle::with_raw_display_handle`.
//! Call [`shutdown`] on the UI thread before the event loop ends (or from `App::on_display_close`)
//! to tear down the shared display substrate while winit's display is still alive.

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
            let applied = wayland::apply(window, desired);
            let mut state = lock(&self.inner.state);
            if state
                .attached
                .as_ref()
                .is_some_and(|attached| attached.key() == window.key())
                && state.desired == desired
            {
                state.applied = applied.then_some(desired);
            }
        }
        protocol
    }

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

        let applied = wayland::apply(&window, enabled);
        let mut state = lock(&self.inner.state);
        if state
            .attached
            .as_ref()
            .is_some_and(|attached| attached.key() == window.key())
            && state.desired == enabled
        {
            state.applied = applied.then_some(enabled);
        }
    }

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
