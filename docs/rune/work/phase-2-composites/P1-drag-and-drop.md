# P1 Drag and drop (Wayland and X11)

| | |
| --- | --- |
| Phase / lane | 2 · Transfer (the long pole) |
| Plan | §4 P1, `operations/drag-and-drop/*` |
| Depends on | P0, W3 |
| Unblocks | X2, M4 |
| Owns | `crates/strata-platform/src/dnd/`, `crates/strata-platform/examples/dnd.rs` |
| Feature | `dnd` |
| Branch | `feat/<issue>-p1-drag-and-drop` |

## Goal

System drag and drop on Linux, for any Rune app. The API mirrors Rune's `drag_out` and
`on_drop_files`, with no Strata names.

## Scope

- **Drag source:** `drag_source(|s| DragItem)` on an element. It starts at Rune's threshold,
  offers several MIME types (through P0's sources), and negotiates the action (copy, move,
  link, ask). The drag icon comes from an element snapshot, or a given image. It reports the
  outcome.
- **Drop target:** `on_drop(|s, DropData, tx|)` for arbitrary MIME types, plus hover, move
  (with a position on every motion event, in the element's coordinates) and reject hooks.
  The chosen action follows Ctrl and Shift.
- **In-process fast path:** a drag between this process's own windows carries a typed
  payload, not serialized URIs. Apps register the payload type.
- Wayland (`wl_data_device`) and X11 (XDND) behind the same API.
- Module docs that link the Rune issues (S5) and say what Rune lacks.

## Tests

- Under W3, with the peer: drag out with three MIME types and check what the peer reads;
  drop in `text/uri-list`, `text/plain`, `text/x-moz-url`, `image/png` and HTML; action
  negotiation with each modifier; a cancelled drag. On Wayland and X11.
- An in-process drag between two windows carries the typed payload.

## Done when

The peer tests pass in CI. A hand test with Nautilus and Dolphin, both directions, on Wayland
and X11, is recorded in the PR. This is the platform half of M4.
