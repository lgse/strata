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
