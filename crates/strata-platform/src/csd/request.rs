//! What the app asks of the window, and what came of it.

use super::Edge;

/// A request to the window or its window manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Close,
    Minimize,
    ToggleMaximize,
    /// An interactive resize from an edge, while the button is held.
    Resize(Edge),
    /// The window manager's window menu, at the pointer.
    WindowMenu,
}

/// How a request went out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Through Rune's `Tx`.
    Rune,
    /// Through this module's X11 connection.
    X11,
    /// Nowhere: no native window (a headless run or a test). Recorded only.
    Headless,
}

/// Why a request can't go out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    /// Rune can't start a resize of a borderless Wayland window. Draft U6, not filed.
    RuneResize,
    /// Rune ignores minimise for a borderless window, and Wayland offers no other path. Draft
    /// U7, not filed.
    RuneMinimize,
    /// Rune has no window menu on Wayland. Draft U8, not filed.
    RuneWindowMenu,
    /// The X11 window manager says it can't: it doesn't list the request in `_NET_SUPPORTED`,
    /// or `_NET_WM_ALLOWED_ACTIONS` leaves it out for this window.
    WindowManager,
    /// The X11 window manager doesn't say whether it has this (no `_NET_SUPPORTED`), and a
    /// request it doesn't know means nothing to it.
    NotKnown,
    /// This module's own X11 connection failed, so nothing reaches the window manager. Draft
    /// U9 (not filed) would let it use winit's.
    NoConnection,
    /// The native frame does this itself.
    NativeFrame,
    /// No native window is attached yet ([`attach`](super::attach)).
    NoWindow,
}

impl Gap {
    /// The Rune issue draft that would close this gap
    /// (`docs/decisions/S5-upstream.md`; drafts, not filed).
    pub fn rune_draft(self) -> Option<&'static str> {
        match self {
            Gap::RuneResize => Some("U6"),
            Gap::RuneMinimize => Some("U7"),
            Gap::RuneWindowMenu => Some("U8"),
            Gap::NoConnection => Some("U9"),
            Gap::WindowManager | Gap::NotKnown | Gap::NativeFrame | Gap::NoWindow => None,
        }
    }
}

/// What came of a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent {
        request: Request,
        route: Route,
    },
    Unsupported {
        request: Request,
        gap: Gap,
    },
    /// The X11 connection failed.
    Failed {
        request: Request,
        error: String,
    },
}

impl Outcome {
    pub fn request(&self) -> Request {
        match self {
            Outcome::Sent { request, .. }
            | Outcome::Unsupported { request, .. }
            | Outcome::Failed { request, .. } => *request,
        }
    }
}
