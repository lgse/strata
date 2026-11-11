//! S3's three owner decisions (`docs/decisions/S3-foreign-parent.md`, "Owner decisions"),
//! **ruled by the owner on 2026-11-10**. The rulings follow S3's recommendations, and the code
//! for them is kept here so a later change touches only this file.
//!
//! - **Decision 1** (libwayland below 1.23): [`without_winit_display`]. Ubuntu 24.04 is
//!   dropped and libwayland 1.23 is the minimum supported version, so this path runs only in
//!   an unsupported environment.
//! - **Decision 2** (`x11:` parents for a Wayland window): [`x11_parent_on_wayland`].
//! - **Decision 3** (no activation token): nothing here. A parentless window is placed and
//!   focused by the compositor's policy (option a), and the caller (Strata) also calls
//!   `WindowHandle::focus` for it (option b). The shim makes no `focus()` call.

use super::outcome::{Fallback, Outcome, Reason, WindowSystem};

/// S3 decision 1, ruled: on libwayland below 1.23 winit's display is out of reach, and the
/// window opens parentless. libwayland 1.23 is the minimum supported version, so this is the
/// graceful behaviour of an unsupported environment, not a supported fallback. Option b would
/// have held the shim until Rune exposes the display (U9); option c (reading libwayland's
/// private proxy layout) isn't to ship.
pub(super) fn without_winit_display() -> Outcome {
    Outcome::parentless(Reason::NoDisplay, Fallback::Compositor)
}

/// S3 decision 2, ruled (option a): a Wayland window can't be transient for an X11 window, so
/// it opens parentless, as a GTK portal dialog does today. Option b, running the window in a
/// helper process on Xwayland, is the recorded upgrade path; option c would ask Rune for an
/// Xwayland window.
pub(super) fn x11_parent_on_wayland() -> Outcome {
    Outcome::parentless(
        Reason::Mismatch {
            parent: WindowSystem::X11,
            window: WindowSystem::Wayland,
        },
        Fallback::Compositor,
    )
}
