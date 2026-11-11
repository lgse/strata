# P5 Desktop settings

| | |
| --- | --- |
| Phase / lane | 1 · Desktop |
| Plan | §4 P5, `settings/themes/portal-text-scaling`, `browser/selection/click-modes` |
| Depends on | W3 (session bus) |
| Unblocks | X1. It feeds C1b (double-click interval) and K2 (text scale) as data, not as a dependency. |
| Owns | `crates/strata-platform/src/settings/` |
| Feature | `settings` |
| Branch | `feat/<issue>-p5-desktop-settings` |

## Goal

Read the desktop settings that Rune does not, and report their changes.

## Scope

- Through `org.freedesktop.portal.Settings`, using zbus: text scaling factor, double-click
  time, cursor blink (on and time), and enable-animations. The keys come from
  `org.gnome.desktop.interface` and `org.gnome.desktop.peripherals`.
- A typed `DesktopSettings` snapshot, plus a change stream delivered on Rune's loop.
- Defaults when the portal is missing or a key is unset. Log each fallback once.
- If S5 finds that Rune's `system_fonts` lacks installed-font enumeration, add it here for K2's
  font picker.

## Tests

- Under W3: a fake `org.freedesktop.portal.Settings` on the private bus returns values,
  emits `SettingChanged`, and the snapshot follows.
- The portal missing gives the defaults.

## Done when

A small example prints the snapshot and every change. It works on GNOME and on Hyprland (hand
test). On Hyprland, `xdg-desktop-portal-gtk` serves the Settings interface:
`xdg-desktop-portal-hyprland` has none (S1, question 4).
