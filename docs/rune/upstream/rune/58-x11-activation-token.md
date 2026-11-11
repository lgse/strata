# 58. X11: `AppHandle::activate(token)` on an open window ignores the startup id

| | |
| --- | --- |
| Kind | New issue (related #22) |
| Drafts | U-P8-1 |
| Affects | P8 (single instance) |
| Retires | P8's note that a forwarded launch may not raise the window on mutter or KWin |
| Checked | At `2a44156`, run under a private `Xvfb :147` (no window manager, own session bus): `repros-head/src/bin/rv_58.rs` opens a window and calls `handle.activate("demo_TIME12345")`, and `rv_58_watch.py` (python-xlib, standing in for the body's x11rb loop; root masks `SubstructureNotify` and `PropertyChange`, as startup-notification messages go with the latter) saw only `_NET_ACTIVE_WINDOW [1, 0, 0, 0, 0]`: source 1, timestamp 0 (`CURRENT_TIME`), and no `_NET_STARTUP_INFO_BEGIN`. Source: off Wayland, `activate` drops the token (`window/lifecycle.rs:135`, `let _ = (event_loop, token); w.focus();`), and winit 0.30.13's `focus_window` (still the lock's winit) sends the request without a timestamp. Gallery: P8's note is still there (`strata-platform/src/single_instance/mod.rs:10`, `tests/session/p8.rs:981`, `README.md:32`). #22 is open; its comment says the token "rides the existing `AppHandle::activate(token)` path", which on X11 drops it. The body's snippet now selects `PROPERTY_CHANGE` too; it had only `SUBSTRUCTURE_NOTIFY`, which would miss the expected remove message. Earlier: by source at `5d421fd` (`lifecycle.rs:382`). |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** X11: `AppHandle::activate(token)` on an open window ignores `DESKTOP_STARTUP_ID`

Context: Strata (lgse/strata), a file manager on Rune, is a single-instance app: it forwards a
second launch's startup id to the running instance, which raises its window with it.

**What's missing** (`2a44156`, by source and run). `activate` drops the token for an open
X11 window (`window/lifecycle.rs:135`, `let _ = (event_loop, token); w.focus();`), and winit's
`focus_window` sends `_NET_ACTIVE_WINDOW` with source 1 and `CURRENT_TIME`. No
startup-notification remove is sent; winit sends one only while it creates a window. mutter
and KWin may refuse a timestamp-less request from an application and only mark the window
urgent, and the launcher's busy cursor stays until it times out.

**Repro.** Under `Xvfb :99`, run a Rune app with its window open and, from another thread, call
`handle.activate("demo_TIME12345")`. Watch the root window:

```rust
// Beside the app: an x11rb client listening on the root window.
use x11rb::{connection::Connection, protocol::{Event, xproto::*}};
let (conn, screen) = x11rb::connect(None)?;
let root = conn.setup().roots[screen].root;
conn.change_window_attributes(root, &ChangeWindowAttributesAux::new()
    .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE))?;
conn.flush()?;
loop {
    if let Event::ClientMessage(m) = conn.wait_for_event()? {
        let name = conn.get_atom_name(m.type_)?.reply()?.name;
        println!("{} {:?}", String::from_utf8_lossy(&name), m.data.as_data32());
    }
}
```

At `2a44156`, under `Xvfb` with no window manager, a watcher on the root window with these
masks sees only `_NET_ACTIVE_WINDOW [1, 0, 0, 0, 0]`.

Expected, as GTK does: `_NET_ACTIVE_WINDOW` with `data[1]` = 12345 (the id's `_TIME`), and
`_NET_STARTUP_INFO_BEGIN` "remove: ID=demo_TIME12345".

**Ask.** On X11, `activate(token)` for an open window takes the `_TIME` timestamp from the
startup id for `_NET_ACTIVE_WINDOW`, and broadcasts the startup id's "remove" message.

Related: #22 (single instance on Linux).
