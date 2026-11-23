// SPDX-License-Identifier: MIT

use super::outcome::{Fallback, Outcome, Reason, WindowSystem};

pub(super) fn without_winit_display() -> Outcome {
    Outcome::parentless(Reason::NoDisplay, Fallback::Compositor)
}

pub(super) fn x11_parent_on_wayland() -> Outcome {
    Outcome::parentless(
        Reason::Mismatch {
            parent: WindowSystem::X11,
            window: WindowSystem::Wayland,
        },
        Fallback::Compositor,
    )
}
