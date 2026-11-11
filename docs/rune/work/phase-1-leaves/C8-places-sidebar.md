# C8 PlacesSidebar

| | |
| --- | --- |
| Phase / lane | 1 · Chrome |
| Plan | §4 C8, `browser/sidebar`, `.../pins`, `.../pins/drag-to-pin`, `.../rail`, `devices/volumes/drive-order` |
| Depends on | W1 (drop zones), W2 |
| Unblocks | C23 (Trash target), X2, M3 |
| Owns | `crates/strata-components/src/sidebar/`, `scenes/c8.rs`, `strata-kit/src/slots/sidebar.rs` |
| Branch | `feat/<issue>-c8-places-sidebar` |

## Goal

rune-kit `Sidebar` with Strata's three widths, pin insertion, device rows and the Trash
target.

## Scope

- Full, icon rail and hidden steps (`browser/sidebar/rail`). The rail is new: icons with
  tooltips and the same keyboard order.
- Collapsible PINNED and DEVICES sections.
- PINNED reorders with `SidebarRow::reorder`. Insertion gaps between pins take internal and
  external folder drops (W1 `DropZone` per gap), with an open-gap animation.
- Device rows: eject and unlock actions, mounted state, a usage bar if Strata shows one, and
  drive dividers in drive order. Devices arrive as data.
- The Trash row is a drop target and also publishes a stable motion key for C23's flight.

## Tests

Scripts: each step of full → rail → hidden, section collapse, pin reorder by keyboard and
drag, a drop into a gap (an intent with its index), eject. A11y golden: sections are groups,
pins announce their position in the set, and device actions are named. Scenes: full and rail,
with devices, with a drop gap open. All in light and dark.

## Done when

The sidebar scripts pass headless.
