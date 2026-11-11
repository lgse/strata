# P4 Foreign parenting for the portal chooser

| | |
| --- | --- |
| Phase / lane | 2 · Desktop |
| Plan | §4 P4, `integration/portal-file-chooser/*` |
| Depends on | S3 (results per compositor), P0 (display access) |
| Unblocks | M6 (Strata's chooser window) |
| Owns | `crates/strata-platform/src/foreign_parent/` |
| Feature | `foreign-parent` |
| Branch | `feat/<issue>-p4-foreign-parent` |

## Goal

Make a Rune window transient for another app's window, from the portal's parent handle.

## Scope

- Parse `wayland:<handle>` and `x11:<xid>`.
- Wayland: `zxdg_importer_v2`, `import_toplevel` and `set_parent_of` on the Rune window's
  surface. X11: `WM_TRANSIENT_FOR`, plus the window type and state hints.
- The fallback chosen per compositor in S3, reported to the caller so it can log it.
- Hyprland placement (`.../window-placement`) stays in Strata.

## Tests

Under W3: a peer toplevel exports a handle, our window imports it, and the compositor reports
the parent (sway's IPC tree). On Xwayland, the property is set.

## Done when

A hand test on Hyprland, GNOME, KDE and sway matches S3's table, and is recorded in the PR.
