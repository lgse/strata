//! Which frame a window opens with, and which backend winit will pick.

use rune::WindowOptions;

/// What the app asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// What this module recommends at the pinned Rune rev: borderless on Wayland and X11.
    #[default]
    Auto,
    /// Always borderless.
    Borderless,
    /// Keep the native frame: the server's title bar, or winit's own frame where the compositor
    /// draws none (GNOME on Wayland).
    Native,
}

/// The frame a window opens with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// No native frame: the app draws its header, and this module moves and resizes the window.
    Borderless,
    /// The native frame moves and resizes the window.
    Native,
}

impl Policy {
    /// The frame this policy opens. `Auto` is borderless on every backend: on Wayland that
    /// means no edge resize, minimise or window menu until Rune ships U6–U8 (module docs).
    pub fn frame(self) -> Frame {
        match self {
            Policy::Auto | Policy::Borderless => Frame::Borderless,
            Policy::Native => Frame::Native,
        }
    }
}

/// `base` with its decorations set for `frame`.
pub fn window_options(frame: Frame, base: WindowOptions) -> WindowOptions {
    WindowOptions {
        borderless: frame == Frame::Borderless,
        ..base
    }
}

/// The native window system a window lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Wayland,
    X11,
    /// No native window this module knows: a headless run, a test, or a window not attached
    /// yet. Requests are recorded in their [`Outcome`](super::Outcome) and go nowhere.
    Headless,
}

impl Backend {
    /// The backend winit 0.30 picks from the environment: Wayland when `WAYLAND_DISPLAY` or
    /// `WAYLAND_SOCKET` is set and not empty, else X11 when `DISPLAY` is, else none.
    /// [`attach`](super::attach) confirms it from the window once it exists.
    pub fn from_env() -> Backend {
        let var = |name| std::env::var(name).ok();
        pick(
            var("WAYLAND_DISPLAY").as_deref(),
            var("WAYLAND_SOCKET").as_deref(),
            var("DISPLAY").as_deref(),
        )
    }
}

/// winit 0.30.13's order (`platform_impl/linux/mod.rs`): Wayland first, since an X server can
/// run under Wayland too.
pub(super) fn pick(
    wayland_display: Option<&str>,
    wayland_socket: Option<&str>,
    display: Option<&str>,
) -> Backend {
    let set = |v: Option<&str>| v.is_some_and(|v| !v.is_empty());
    if set(wayland_display) || set(wayland_socket) {
        Backend::Wayland
    } else if set(display) {
        Backend::X11
    } else {
        Backend::Headless
    }
}
