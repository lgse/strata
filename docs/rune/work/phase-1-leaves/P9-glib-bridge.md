# P9 GLib bridge

| | |
| --- | --- |
| Phase / lane | 1 · Desktop |
| Plan | §4 P9, §7 phase 0 (the prototype is this WP's first step) |
| Depends on | W0 |
| Unblocks | C26, X1. Strata's devices, network and monitor code. |
| Owns | `crates/strata-platform/src/glib_bridge/` |
| Feature | `glib-bridge` |
| Branch | `feat/<issue>-p9-glib-bridge` |

## Goal

Run GIO async work, such as volumes, mounts, monitors, AppInfo and GVfs, without GTK. Use a
`glib::MainContext` on its own thread, and marshal results into Rune.

## Scope

- Start a dedicated thread that owns a `MainContext` and `MainLoop`, and shut it down cleanly
  when the app quits.
- `spawn(future)` runs a GIO future on that context. Its result is delivered on Rune's loop
  through `AppHandle::update` or the setup `Waker`.
- A signal helper: connect a GObject signal on the GLib thread, and receive its arguments on
  Rune's loop.
- A request and response path for signal handlers that must answer, such as
  `MountOperation::ask-password`. The handler waits on the GLib side while the Rune side
  answers. C26 builds on this.
- Rules in the module docs for which side may touch which objects. GObjects are not `Send`
  across the bridge.

## Tests

- A `gio::File` query on a temp file returns on Rune's loop, in a headless `Runtime`.
- A `gio::FileMonitor` change arrives.
- A request and response round-trip, with a fake handler.

## Done when

The tests pass headless, and the thread exits when the `Runtime` drops. A note in the module
docs lists what Strata moves onto it.
