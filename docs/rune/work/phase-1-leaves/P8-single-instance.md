# P8 Single instance and activation

| | |
| --- | --- |
| Phase / lane | 1 · Desktop |
| Plan | §4 P8, `integration/file-manager-interface`, `browser/navigation/startup-arguments` |
| Depends on | W3 (session bus) |
| Unblocks | X1 |
| Owns | `crates/strata-platform/src/single_instance/` |
| Feature | `single-instance` |
| Branch | `feat/<issue>-p8-single-instance` |

## Goal

Replace what `gtk::Application` gave Strata: one primary process that owns a well-known name,
while later launches hand over their arguments and exit.

## Scope

- At start, try to own a caller-given well-known name with zbus. If it is taken, forward argv,
  the working directory and the activation token (`XDG_ACTIVATION_TOKEN`, or
  `DESKTOP_STARTUP_ID` on X11) to the primary, then exit with its status.
- The primary gets each forwarded launch on Rune's loop, and can open a window with
  `rune::open_window` and focus it with `activate(token)`.
- `org.freedesktop.Application` (`Activate`, `Open`, `ActivateAction`) for D-Bus activation.
  FileManager1 and the portal's own interfaces stay in Strata, on the same connection, so the
  API exposes it.
- A race between two launches at the same moment ends with one primary.

## Tests

Under W3's private bus: a second process forwards its argv, the primary sees it, and the
second exits 0. Two launched at once end with one primary.

## Done when

The example `single-instance-demo a b` followed by a second `single-instance-demo c` opens a
second window in the first process, focused, on Hyprland and GNOME (hand test).
