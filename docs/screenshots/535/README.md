# Issue #535: file chooser sizing in split windows

`reporter.png` is the screenshot supplied by the maintainer from the Discord
report by `fadilasif`. It shows a floating file chooser extending beyond the
browser's split-window area. The reporter says the chooser has the same size
when opened from a fullscreen application. Distribution, compositor, and exact
window dimensions remain unconfirmed.

## Controlled comparison

The `monitor-fallback-*.png` and `split-window-*.png` images exercise the same
chooser fixture on a private 1440 × 900 Xvfb display, in Columns, Icons, and List.

- Monitor fallback (the previous sizing policy): 1000 × 680 logical pixels.
- With a simulated 960 × 540 application size hint: 768 × 460 logical pixels.

These are layout regression captures, not a reproduction on the reporter's
compositor. Both are rendered from the new build, with and without the hint.
The separate socket tests exercise read-only Hyprland IPC using temporary Unix
sockets, including ambiguity, invalid responses, unavailable sockets, oversized
responses, and timeouts. No test connects to the user's compositor.

To regenerate the layout captures on a private display:

```bash
xvfb-run -a env -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS \
  GDK_BACKEND=x11 GTK_A11Y=none NO_AT_BRIDGE=1 STRATA_REQUIRE_GTK_TESTS=1 \
  STRATA_CHOOSER_SIZE_VISUALS="$PWD/docs/screenshots/535" \
  cargo test --all-targets --all-features -- --exact \
  ui::chooser::tests::sizing::application_size_hint_survives_presentation_in_every_view
```

## Monitor-centering follow-up

`center-before.png` and `center-after.png` are actual Wayland captures from an
isolated Hyprland 0.56.2 instance, not simulated positions. The before binary is
from `4ad1ef9` (the sizing-only fix); the after binary includes the centering
follow-up. Both ran against the same host GTK 4.22 toolkit.

The 1920 × 1080 virtual monitor contained a 900 × 520 calling application at
(20, 30). The fixture exported a real Wayland parent handle and made portal
OpenFile, SaveFile, and SaveFiles requests on a private D-Bus session.

- Before: the 720 × 460 chooser opened at (110, 60), centered on the caller.
- After: it opened at (600, 310), centered on the monitor at (960, 540).
- Size and transiency were preserved. OpenFile also passed after a compositor
  config reload, and the normal file manager retained its separate identity and
  tiled behavior.

`centering-lua-results.json` and `centering-legacy-results.json` record the
asserted geometry for both configuration parsers. To exercise this manually,
invoke a portal Open or Save dialog from an off-center application window, then
compare its center with the monitor's center. Repeat after reloading Hyprland's
configuration, and check that regular Strata windows retain their placement.

The compositor ran inside Bubblewrap with private HOME/XDG directories, no
access to desktop sockets or DRM card devices, and only a GPU render node. Cage
with a headless wlroots backend supplied a nested Wayland renderer; Hyprland's
own virtual output supplied the test monitor. A test-only preload workaround
clamped Aquamarine 0.14's nested `wl_compositor`/`xdg_wm_base` bindings to version
5 because it otherwise requests version 6 from a version-5 server. The shim was
loaded only by the nested compositor, never by Strata, and is not shipped.

## Implementation scope

GTK already clamps its default dimensions to compositor-provided bounds, subject
to widget minimum sizes. Another generic bounds clamp would not identify the
requesting application's dimensions.

The fix obtains a best-effort size hint on Hyprland when a Wayland parent and
application ID are supplied and exactly one window matches that app ID. It does
not guess from keyboard focus. Other compositors, ambiguous app windows, missing
or differently named IDs, and unavailable IPC retain monitor-based sizing. The
chooser no longer restores monitor-derived defaults on monitor entry.

References:

- https://github.com/lgse/strata/issues/535
- https://github.com/GNOME/gtk/blob/4.22.2/gtk/gtkwindow.c
- https://wayland.app/protocols/xdg-foreign-unstable-v2
