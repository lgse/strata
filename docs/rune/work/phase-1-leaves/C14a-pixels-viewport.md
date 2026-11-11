# C14a Pixels viewport

| | |
| --- | --- |
| Phase / lane | 1 · Preview |
| Plan | §4 C14 (image view with zoom and pan), shared by C12 and C13 |
| Depends on | W2 |
| Unblocks | C12, C13, C14b |
| Owns | `crates/strata-components/src/viewers/viewport/`, `scenes/c14a.rs` |
| Branch | `feat/<issue>-c14a-pixels-viewport` |

## Goal

One zoomable, pannable surface over a `rune::Pixels`, used by the image view, PDF pages, the
video surface and the model view.

## Scope

- Fit, fill and 1:1 modes. Zoom with `on_wheel` (with Ctrl) and `on_magnify`, about the
  pointer. Pan with drag and with the wheel. Keyboard: `+`, `-`, `0` and arrows.
- Zoom steps, plus a "zoom changed" report so hosts (C12) can re-render at the new scale.
- The `Pixels` handle updates in place with no view commit, which matters for video.
- Checkerboard behind transparency, from Kit tokens.

## Tests

Scripts: wheel zoom about the pointer, pinch, fit after a resize, pan limits, and the
keyboard. A11y golden: an image with a name and the zoom level. Scenes: fit, 1:1 and zoomed,
in light and dark.

## Done when

C12's and C13's owners review the API.
