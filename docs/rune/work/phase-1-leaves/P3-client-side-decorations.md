# P3 Client-side decorations on Linux

| | |
| --- | --- |
| Phase / lane | 1 · Desktop |
| Plan | §4 P3, `app/window` |
| Depends on | S1, W3 |
| Unblocks | X1 |
| Owns | `crates/strata-platform/src/csd/`, `crates/strata-platform/examples/csd.rs` |
| Feature | `csd` |
| Branch | `feat/<issue>-p3-client-side-decorations` |

## Goal

Let a Rune app on Linux draw its own header and still behave like a window: it can be moved,
resized from its edges, maximised by double-click, and given its window menu.

## Scope

- Borderless window setup, keeping server-side decorations where the compositor offers them
  and the app asks for them.
- Resize edges and corners: hit-test bands that call winit `drag_resize_window` with the
  right direction, and set the matching cursors.
- Window buttons as plain handlers (close, minimise, maximise or restore). The look is the
  app's job. X1 draws them with strata-kit.
- Double-click on a drag region to maximise. The interval comes from P5 when present, with a
  default otherwise.
- The window menu (`show_window_menu`) on secondary click in the header.
- Hyprland draws no decorations either way. Check that nothing breaks there.
- Module docs that link the Rune issue (S5) and say what Rune lacks.

## Tests

- Headless: a scripted press on an edge band records the resize direction, and a double-click
  records a maximise request.
- Under W3: the example window resizes from each edge under sway.

## Done when

The example app, with no rune-kit, drags, resizes, maximises and opens its menu on Hyprland,
GNOME and X11. Record the hand test in the PR.

**Narrowed by the owner (P3 planning).** `Policy::Auto` is borderless on Wayland and X11.

- **Wayland:** the window drags and maximises through Rune. It has no edge resize, minimise
  or window menu until Rune ships U6–U8. Those requests answer Unsupported with their draft.
- **X11:** a shim on a second connection sends `_NET_WM_MOVERESIZE`, `WM_CHANGE_STATE` and
  `_GTK_SHOW_WINDOW_MENU`, the last only where `_NET_SUPPORTED` lists it. Whether this works
  on mutter and KWin is a hand test, still pending.
- **Also decided:** there is no in-app resize fallback, corners use `Cursor::Move`, and X1
  builds Strata's header component.
- **Parity:** the regressions against GTK Strata are deliberate. On Wayland, no edge resize,
  minimise or window menu. The header's double-click interval is Rune's fixed 0.5 s (U13), not
  P5's setting.

`csd`'s module docs, under "Where it stands", record this and S1's constraints.
