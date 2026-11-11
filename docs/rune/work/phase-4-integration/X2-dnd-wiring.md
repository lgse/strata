# X2 Drag-and-drop wiring

| | |
| --- | --- |
| Phase / lane | 4 · Integration (the end of the long pole) |
| Plan | §7 phase 4 exit check, `operations/drag-and-drop/*` |
| Depends on | P1, C2, C3, C4, C6, C7, C8 |
| Unblocks | M4 |
| Owns | `crates/strata-gallery/examples/dnd.rs` (or a gallery demo module) |
| Branch | `feat/<issue>-x2-dnd-wiring` |

## Goal

Join W1's drag and drop intents to P1, and show every drop target working across processes.

## Scope

- An adapter, written in the demo so Strata can copy it, that maps:
  - `DragPayload` onto P1's `DragItem`, offering `text/uri-list`, the GNOME and KDE formats,
    and the in-process typed payload;
  - P1 drops onto each component's `DropZone` callbacks, with the action from Ctrl and Shift;
  - P1 motion events onto the hover state, so folder rows, crumbs, tabs and sidebar gaps
    light up;
  - drags across Columns, so the strip auto-scrolls (C2).
- In-process drags between two demo windows use the typed fast path.
- If the adapter turns out to be generic over any Rune app, move it into
  `strata-components` behind a `platform` feature, with the owner's agreement. That
  agreement relaxes rule 1 in the index.

## Tests

Under W3, with the peer: drag from each view out, and drop into a folder row, a crumb, a tab,
New tab and a sidebar gap, on Wayland and X11. In-process between two windows. Hand tests with
Nautilus and Dolphin are recorded in the PR.

## Done when

The DnD half of M4: round-trips with Nautilus and Dolphin, on Wayland and X11.
