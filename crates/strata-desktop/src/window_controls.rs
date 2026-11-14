// SPDX-License-Identifier: MIT

//! Initial window-control visibility for desktop sessions.

/// Hide window controls on recognized tiling desktops; otherwise keep them discoverable.
/// Saved application preferences take precedence over this initial default.
pub fn default_window_buttons_visible() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    buttons_visible_for_desktop(&desktop)
}

fn buttons_visible_for_desktop(desktop: &str) -> bool {
    const TILING_DESKTOPS: [&str; 4] = ["Hyprland", "Sway", "i3", "niri"];
    !desktop.split(':').any(|name| {
        TILING_DESKTOPS
            .iter()
            .any(|tiler| name.trim().eq_ignore_ascii_case(tiler))
    })
}

#[cfg(test)]
mod tests;
