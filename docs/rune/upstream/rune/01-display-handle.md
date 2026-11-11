# 01. Display handle, its lifetime, and an every-turn hook

| | |
| --- | --- |
| Kind | New issue. File first: 05, 06, 07 and 23 link it. |
| Drafts | U9 (S5; from S1 R7, S2 R4, S3) |
| Affects | P0, P1, P2, P3 (X11), P4 |
| Retires | P0's `dlsym("wl_proxy_get_display")` recovery and its reader thread, the second X11 connection |
| Checked | Re-checked at `2a44156` by source: `WindowHandle::with_raw_handle` (`window/handle.rs:148`) is still the only accessor, and it yields the window handle only. Rune's own `platform/activation.rs:51` still reads winit's display. `App::turn_scope` (`app.rs:441`, `6d8d32bd`, PR #392) is new since the pin. It wraps `about_to_wait` (`window/any_host.rs:59`), but #439 (open) shows that winit 0.30 skips that turn when it dispatched none of its own events, so `turn_scope` doesn't cover the every-turn hook. The ask is reworded to say so. strata-platform's workaround is still there (`transfer/wayland/display.rs`, `transfer/wayland/reader.rs`, `csd/x11.rs`'s own connection), and `mise run test no_wl_proxy_get_display_is_unsupported` passed (1 test). Earlier: by source at `5d421fd` and `04b38ae`. The repro needs a window: S1–S3 measured it at `78d22ed`, and it was not re-run. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10). The every-turn hook ask is reworded around `App::turn_scope` and #439; it is not dropped. |

Paste from here.

---

**Title:** Linux: `WindowHandle` gives no display handle, no warning before the display closes, and no every-turn hook

Context: Strata (lgse/strata), a file manager on Rune, moving from GTK 4. It needs Wayland protocols
Rune doesn't bind (`wl_data_device` for drag and drop, data-control for the clipboard,
`zxdg_importer_v2` for portal dialogs) and X11 requests on winit's own connection.

**What's missing** (Rune `dev` `5d421fd`, unchanged at `2a44156`):
- `WindowHandle::with_raw_handle` (`window/handle.rs:148`) yields the window handle only: the
  `wl_surface` on Wayland, the Xlib window on X11. There is no display handle, though Rune's own
  `platform/activation.rs` binds a protocol on winit's display.
- Nothing says how long winit's display lives. Proxies bound on it and dropped after `run`
  returns crashed the process (`wl_proxy_destroy` from the backend's drop) in 5 of 5 runs.
- A second event queue on winit's display must be dispatched on the UI thread whenever a read
  leaves events in it. `App::turn_scope` (#392) runs around each turn, `about_to_wait`
  included, but winit skips the turn when it dispatched none of its own events (#439). So a
  queue dispatched there waits for an unrelated wake, as a thread that only polls the fd does:
  events waited 123–279 ms (p99).

**Repro** (needs any Wayland session, or Xvfb for X11; no input):

```rust
use rune::{App, AppSpec, Cx, El, col};
#[derive(Default)] struct S;
fn view(_: &S, _: &mut Cx<S>) -> El { col().w_full().h_full() }
fn main() {
    App::new(AppSpec::new("display", (320.0, 200.0), S::default, view))
        .on_window(|w| println!("{:?}", w.with_raw_handle(|h| format!("{:?}", h.as_raw()))))
        .run();
}
```

Wayland prints `Ok("Wayland(WaylandWindowHandle { surface: 0x… })")`; X11 prints
`Ok("Xlib(XlibWindowHandle { window: …, visual_id: … })")`. Neither gives the display. (Measured
at `78d22ed`; the accessor is unchanged since.)

**Ask.**
- `WindowHandle::with_raw_display_handle(|d| ..)`: winit's `wl_display` on Wayland, its Xlib
  display or xcb connection on X11.
- A stated lifetime for it, or a callback on the UI thread before winit closes it.
- Optionally, a turn on the UI thread whenever a read of winit's display leaves events in an
  app's queue, as #439's fix does for Rune's layer overlays, and a hook in that turn that can
  ask for a frame.

**Why.** Without these, a downstream crate recovers the display from the surface with
`wl_proxy_get_display` through `dlsym` (libwayland 1.23 or later only; Ubuntu 24.04 ships 1.22),
runs a reader thread that joins libwayland's read protocol, and opens a second X connection,
which can't release winit's pointer grab that `_NET_WM_MOVERESIZE` needs. Drag and drop,
clipboard, portal dialog parenting and X11 window moves all ride on that.

Related: #171, #424 (drag and drop on Linux), #139 (a covered window's loop stalls), #439 (the
same missed wake, for layer overlays).
