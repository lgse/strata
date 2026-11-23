// SPDX-License-Identifier: MIT

//! What [`super::apply`] did, for the caller to log and to place the window by.

use std::fmt;

/// Wayland or X11: of a parent, or of the app's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowSystem {
    Wayland,
    X11,
}

impl fmt::Display for WindowSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WindowSystem::Wayland => "Wayland",
            WindowSystem::X11 => "X11",
        })
    }
}

/// The result of [`super::apply`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// The window is transient for the parent.
    Parented { system: WindowSystem, late: bool },
    /// The window has no parent: why, and what places it instead.
    Parentless { reason: Reason, fallback: Fallback },
}

impl Outcome {
    pub fn is_parented(&self) -> bool {
        matches!(self, Outcome::Parented { .. })
    }

    pub(super) fn parentless(reason: Reason, fallback: Fallback) -> Outcome {
        Outcome::Parentless { reason, fallback }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Parented { system, late } => {
                write!(f, "transient for its {system} parent")?;
                if *late {
                    f.write_str(", set after the window mapped")?;
                }
                Ok(())
            }
            Outcome::Parentless { reason, fallback } => {
                write!(f, "parentless: {reason}; {fallback}")
            }
        }
    }
}

/// Why a window was left without a parent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// The request named no parent.
    NoParent,
    Mismatch {
        parent: WindowSystem,
        window: WindowSystem,
    },
    /// The compositor has no `zxdg_importer_v2` (a v1-only compositor counts as none).
    NoImporter,
    NoDisplay,
    Destroyed,
    BadXid,
    /// The app's window was closed before the call.
    WindowClosed,
    /// Neither a Wayland nor an X11 window (the raw handle's kind).
    NotLinux(String),
    /// The connection or a request failed.
    Failed(String),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::NoParent => f.write_str("the request named no parent"),
            Reason::Mismatch { parent, window } => {
                write!(f, "the parent is on {parent} and the window on {window}")
            }
            Reason::NoImporter => f.write_str("the compositor has no zxdg_importer_v2"),
            Reason::NoDisplay => f.write_str(
                "libwayland < 1.23: no wl_proxy_get_display, so winit's display is out of reach",
            ),
            Reason::Destroyed => f.write_str("the compositor refused the handle"),
            Reason::BadXid => f.write_str("the parent X11 id names no window that can be a parent"),
            Reason::WindowClosed => f.write_str("the window is closed"),
            Reason::NotLinux(kind) => write!(f, "not a Wayland or X11 window: {kind}"),
            Reason::Failed(e) => write!(f, "failed: {e}"),
        }
    }
}

/// What places a parentless window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Fallback {
    Compositor,
    /// The shim asked for the window to be centred on its screen (`WindowHandle::center`).
    Centred,
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Fallback::Compositor => "the compositor places it",
            Fallback::Centred => "centred on the screen",
        })
    }
}
