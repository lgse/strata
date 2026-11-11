# 23. A window transient for another app's window (or one of the app's own), applied before it maps

| | |
| --- | --- |
| Kind | New issue; links 01 |
| Drafts | U11 (S5, from S3) with P4's two amendments (modal on Wayland, a window transient for the app's own window) |
| Affects | P4 (foreign parent), the portal file chooser |
| Retires | P4's `zxdg_importer_v2` import on winit's display and its own-connection X11 hints; the "opened hidden, one window per request" rule |
| Checked | Re-checked at `2a44156`, by source: `window/create.rs:13` still computes `visible = !launcher && (self.started \|\| self.options.visible)`, the window is created (mapped when visible) at `:44`, and `App::on_window` runs after it (`:243`); no transient, xdg-foreign or `xdg_wm_dialog_v1` API anywhere in `crates/rune/src`. The repro needs a compositor and was not re-run. In the gallery, P4's workaround is still there (`zxdg_importer_v2` on winit's display, `foreign_parent/wayland.rs:1`; the hidden one-window-per-request rule, `foreign_parent/mod.rs:8–9`, `:26`), and `mise run test-platform p4::` passed in a private headless sway (12, incl. `x11_a_parent_applied_after_the_map_reaches_the_window_manager`). Closest issue: #143 (Rune as a portal client passes `""` as the parent on Wayland), the other direction. Earlier: the same line at `5d421fd` and `04b38ae`; S3's sway measurement at `78d22ed`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** Linux: make a window transient for another app's window, applied before it maps

Context: Strata's (lgse/strata, a file manager on Rune) file chooser is an XDG portal backend. It
gets its caller's window as `wayland:<xdg-foreign handle>` or `x11:<xid>`, and its dialog must be
transient for that window: xdg-foreign v2 `set_parent_of` on Wayland, `WM_TRANSIENT_FOR` on X11.
Rune has no API for either.
Reaching winit's display is a separate gap: (link to 01).

**Even with the display, the parent has to be set before the window maps** (`5d421fd`,
unchanged at `2a44156`):
- On X11, `App::on_window` runs after winit has mapped the window, unless it was created hidden.
- `WindowOptions { visible: false }` holds only on a window's first creation:
  `window/create.rs:13` computes `visible = !launcher && (self.started || self.options.visible)`,
  so a recreated window maps at once.

**Repro** (needs a compositor; headless tooling never creates a native window). The program
opens a hidden window, shows it, closes it (destroying it, with `release_gpu_when_closed`),
reopens it and quits:

```rust
// Cargo.toml: rune, x11rb 0.13
use std::sync::atomic::{AtomicU32, Ordering};
use rune::raw_window_handle::RawWindowHandle;
use rune::{AppSpec, Cx, El, el};
fn view(_: &(), _: &mut Cx<()>) -> El { el() }
static CREATED: AtomicU32 = AtomicU32::new(0);
fn main() {
    let spec = AppSpec::new("transient", (400.0, 300.0), || (), view);
    let hidden = rune::WindowOptions { visible: false, ..Default::default() };
    let app = rune::App::new(spec).args(Vec::new()).keep_running().release_gpu_when_closed().window_options(hidden)
        .on_window(|w| {
            let n = CREATED.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = w.with_raw_handle(|h| match h.as_raw() {
                RawWindowHandle::Wayland(wl) => println!("creation {n}: wayland surface {:p}", wl.surface),
                RawWindowHandle::Xlib(x) => {
                    use x11rb::protocol::xproto::ConnectionExt;
                    let (c, _) = x11rb::connect(None).unwrap();
                    let a = c.get_window_attributes(x.window as u32).unwrap().reply().unwrap();
                    println!("creation {n}: x11 window {:x}, map_state {:?}", x.window, a.map_state);
                }
                _ => {}
            });
            if n == 1 { w.show(); }
        });
    let handle = app.handle();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        handle.close();
        std::thread::sleep(std::time::Duration::from_millis(500));
        handle.open();
        std::thread::sleep(std::time::Duration::from_secs(1));
        handle.quit();
    });
    app.run();
}
```

On X11 (Xwayland under sway, at `78d22ed`): `creation 1: … map_state UNMAPPED`, then
`creation 2: … map_state VIEWABLE`: the second creation ignored `visible: false` and mapped
before `on_window`, so a `WM_TRANSIENT_FOR` set there reaches the window manager after the map.

**Ask.** `WindowOptions::transient_for(parent)` taking a foreign handle (`wayland:…`, `x11:…`)
or another of the app's windows, applied before the window maps on every creation, with an
outcome saying whether the compositor accepted it, and an option to make it modal
(`xdg_wm_dialog_v1` `set_modal` where the compositor has it, `_NET_WM_STATE_MODAL` on X11).
Failing that, a hook that runs before every native window maps, and access to the window's
`xdg_toplevel` (needed for `xdg_wm_dialog_v1`, which `with_raw_handle`'s `wl_surface` can't
reach).

**Why.** A downstream crate imports the handle on winit's display (recovered unsafely, see
(link to 01)), sets X11 hints from a second connection in `on_window` of a window opened
hidden, uses one window per request, can't mark a Wayland dialog modal, and can't parent a
chooser to the app's own window without exporting and re-importing it.
