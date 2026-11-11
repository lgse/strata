# 11. Comment on #309: edge resize from a borderless window, on Wayland

| | |
| --- | --- |
| Kind | Comment on [#309](https://github.com/anomalyco/rune/issues/309) (resize a borderless window from edges the app draws; filed 2026-11-08). S5 drafted U6 as a new issue before it existed. |
| Drafts | U6, its resize half (S5, from S1 R1) |
| Affects | P3, X1 |
| Retires | P3's "no edge resize on Wayland" known limit (owner's P3 decision), and on X11 the `_NET_WM_MOVERESIZE` sent from a second connection |
| Checked | Re-checked at `2a44156` by source. There is still no `drag_resize_window`, `ResizeDirection` or `El::window_resize` anywhere in `crates/`: `.window_drag()` (`el/layout.rs:596`) reaches winit's `drag_window` only (`window/events.rs:494`), and `with_raw_handle` still yields only the window handle (`window/handle.rs:148`). #309 is still open with no comments (checked live 2026-11-10). P3's known limit (`csd/mod.rs:146`, "no edge resize, minimise or window menu" on Wayland) and its X11 `_NET_WM_MOVERESIZE` on its own connection (`csd/x11.rs`) are still there. S1's sway measurement was not re-run. Earlier: the same by source at `5d421fd` and `04b38ae`. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10). Post it as a comment on #309, which is still open. |

Paste from here.

---

Adding Linux evidence from Strata (lgse/strata), a file manager on Rune that draws its own header in
a borderless window.

Under headless sway (Wayland and Xwayland), plain winit 0.30.13's
`Window::drag_resize_window(ResizeDirection::East)`, called from a press 3 px inside the right
edge, grows the window by the pointer's 100 px travel. A Rune window can't do it: nothing calls
`drag_resize_window`, `with_raw_handle` yields only the `wl_surface` (and `xdg_toplevel.resize`
needs winit's toplevel and the press serial), so on Wayland there is no workaround at all. On
X11 an app can send `_NET_WM_MOVERESIZE` from a second connection, but that connection can't
release winit's implicit grab.

The proposed `El::window_resize(WindowEdge)` covers it. On Wayland the press has to reach
winit inside the same event (its serial), as `.window_drag()` does. The corner cursors are #145
and #292.
