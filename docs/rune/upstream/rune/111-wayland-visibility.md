# 111. A Wayland window that is minimised or suspended never reaches `App::on_visibility`

| | |
| --- | --- |
| Kind | New issue |
| Drafts | A21 of the parity plan (`docs/rune/parity/packages/A21.md`), for the window's presence that each browser rechecks its folders by |
| Affects | `strata-app`'s presence (`crates/strata-app/src/shell/presence.rs`): GTK's `presence_of` (`src/ui/presence.rs`) reads a toplevel's `SUSPENDED` and `MINIMIZED` states, which GDK takes on Wayland from `xdg_toplevel`'s `suspended` state, and turns the window's browsers `Hidden`, so they recheck their folders less often. On Rune a minimised Wayland window stays `Visible` |
| Retires | The Wayland half of the "Presence differs from GTK's both ways" item in `A21.md`'s "Open"; no code: Strata already follows `on_visibility` |
| Checked | By reading at `631e8a67` (2026-11-19): `App::on_visibility` (`app.rs`, line 309) is called from `set_live_visible` (`runtime/motion/live_latch.rs`, lines 180 to 188), which the host drives from `WindowEvent::Occluded` (`window/events.rs`, lines 257 to 266; `window/covered_tests.rs`) and from Rune's own hide. Its other occluded route (`window/draw.rs`) is reached only from macOS code. winit 0.30.13, the version `Cargo.lock` pins, documents `Occluded` as "Unsupported" on Wayland (`event.rs`, line 421). It does read `XdgWindowState::SUSPENDED`, but only to keep that state from asking for a resize (`platform_impl/linux/wayland/window/state.rs`, line 388), and reports nothing from it. So on Wayland only Rune's own hide reaches the hook |
| Status | Drafted, not filed: to file at the owner's filing pass |

Paste from here.

---

**Title:** `App::on_visibility` hears nothing when a Wayland window is minimised or suspended

Context: Strata (lgse/strata), a file manager on Rune, is moving from GTK. It polls its open
folders less often while their window is out of view, as its GTK version does, and follows
Rune's `App::on_visibility` for that.

At `631e8a67`, `on_visibility` hears a native window go off screen from winit's
`WindowEvent::Occluded`, and from Rune's own hide. winit 0.30 does not send `Occluded` on
Wayland ("Android / Wayland / Windows / Orbital: Unsupported"); it reads the toplevel's
`suspended` state only to keep it from asking for a resize, and reports nothing from it. Rune
reads nothing else there. A compositor does say when a toplevel stops being seen: xdg-shell (version 6) has the
`suspended` state on `xdg_toplevel.configure`, which a compositor may set for a window that is
minimised or otherwise out of view. GTK 4 reads it as `GDK_TOPLEVEL_STATE_SUSPENDED`.

**Repro sketch** (Wayland, `631e8a67`):
1. Build an `App` with `.on_visibility(|s, shown, _| s.shown = shown)`.
2. Open it under a compositor that sets `suspended` (check with `WAYLAND_DEBUG=1`), and minimise
   the window.
3. Expected: `on_visibility(false)`, then `on_visibility(true)` when it comes back. Actual:
   neither is called.

**Ask.** Report `xdg_toplevel`'s `suspended` state to `App::on_visibility` on Wayland, from
winit once it reads it, or from Rune's own handling of the toplevel's configure until then.
