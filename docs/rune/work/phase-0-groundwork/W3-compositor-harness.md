# W3 Compositor test harness

| | |
| --- | --- |
| Phase / lane | 0 · Transfer and Desktop |
| Plan | §6 (platform shims), §8 (Wayland DnD inside winit) |
| Depends on | W0 |
| Unblocks | P0–P5, P8 |
| Owns | `crates/strata-platform/tests/support/`, `.github/workflows/platform.yml`, a `mise` task |
| Branch | `ci/<issue>-w3-compositor-harness` |

## Goal

Give every platform shim one way to run against a real compositor and a real D-Bus, locally
and in CI. Every P-track needs it, and none should build its own.

## Scope

- A test support module that starts a nested headless sway (`WLR_BACKENDS=headless`) with
  Xwayland, gives each test its own `WAYLAND_DISPLAY` and `DISPLAY`, and tears them down.
  Follow the way Rune checks its layer shell.
- A private session bus for each test (`dbus-run-session`, or a spawned `dbus-daemon`), for
  P5, P8 and the portal tests.
- A peer process: a tiny program, written in Rust with raw `wayland-client` and `x11rb`, that
  can offer and accept drags, set and read the clipboard, and report what it received as
  JSON. It is the stand-in for "a second process", as Strata's
  `tests/qa/foreign_drag_source.py` is.
- Tests that need a compositor are `#[ignore]`d by default and run with
  `mise run test-platform`, which sets up the environment. `mise run test` still never
  touches a display.
- A CI workflow that installs sway, Xwayland and dbus and runs `test-platform`. Hyprland in
  Strata's QEMU images stays a Strata-side job.

## Out of scope

The shims themselves, and GNOME and KDE runs, which S1 and S3 do by hand.

## Done when

A sample test opens a Rune window under headless sway and under Xwayland, and the peer
round-trips a text clipboard with itself. It passes in CI. `mise run check` stays
display-free.
