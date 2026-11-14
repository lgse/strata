// SPDX-License-Identifier: MIT

use super::buttons_visible_for_desktop;

#[test]
fn window_buttons_follow_desktop_names() {
    for (desktop, visible) in [
        ("KDE", true),
        ("GNOME", true),
        ("ubuntu:GNOME", true),
        ("XFCE", true),
        ("X-Cinnamon", true),
        ("Hyprland", false),
        ("Sway", false),
        ("i3", false),
        ("niri", false),
        ("hYpRlAnD", false),
        ("KDE: Sway ", false),
        ("niri:GNOME", false),
        (" : i3 : ", false),
        ("", true),
        ("::", true),
        ("unknown-desktop", true),
        ("sway-like", true),
        ("not-Hyprland", true),
    ] {
        assert_eq!(buttons_visible_for_desktop(desktop), visible, "{desktop:?}");
    }
}
