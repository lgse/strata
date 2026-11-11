# 13. Comment on #313: the window menu on Wayland too, and the titlebar double-click action

| | |
| --- | --- |
| Kind | Comment on [#313](https://github.com/anomalyco/rune/issues/313) (window menu on X11, tiled states) |
| Drafts | U8 (S5, from S1 R3) |
| Affects | P3, X1 |
| Retires | X1's right-click on the header doing nothing on Linux (`csd::Gap::RuneWindowMenu`, `strata-platform/src/csd/request.rs:37`, returned for a borderless Wayland window at `csd/state.rs:321`); the header's double press that always maximises |
| Checked | At `2a44156` (2026-11-10), by source: `chrome::window_menu` (`window/chrome.rs:96`) is still a no-op off Windows, and `titlebar_double_click` (`window/chrome.rs:72–77`) still always maximises off macOS; winit 0.30.13's Wayland `show_window_menu` is implemented (`platform_impl/linux/wayland/window/mod.rs:374`), X11's is empty (`x11/window.rs:1666`). #313 still open, no comments, last updated 2026-11-08 (`gh issue view`). strata-platform's `csd::` tests pass (14, incl. `Gap::RuneWindowMenu` for Wayland). Earlier: same by source at `5d421fd` (`:95`, no point) and `04b38ae`. |
| Status | Ready to file, trimmed: dropped the `wm_capabilities` bullet and X11's `_GTK_SHOW_WINDOW_MENU`: #313's own body asks for both, with the same sway and Hyprland findings (re-verified at 2a44156, 2026-11-10) |

Paste from here.

---

From Strata (lgse/strata), a file manager on Rune that draws its own header: on Wayland the window
menu is missing as well. `chrome::window_menu` (`window/chrome.rs:96` at `2a44156`) does nothing off
Windows, so a right-click on a `.window_drag()` region opens nothing. winit's Wayland
`show_window_menu` already sends `xdg_toplevel.show_window_menu`, which GNOME and KDE expect; Rune
doesn't call it.

One more piece would let a custom header match the desktop: the desktop's titlebar double-click
action (`org.gnome.desktop.wm.preferences action-double-click-titlebar`, also in the Settings
portal), instead of always maximising (`titlebar_double_click`, `window/chrome.rs:72–77` at
`2a44156`).
