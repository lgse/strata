# C1e Collection element

| | |
| --- | --- |
| Phase / lane | 2 · Collection (critical path) |
| Plan | §4 C1 (rows host slots, position restore, performance target), `browser/scrolling` |
| Depends on | C1a. It uses C5, C24 and C25 when they have landed, and leaves an empty slot when they have not. |
| Unblocks | C2, C3, C4, C14b, C23 integration, X2 |
| Owns | `crates/strata-components/src/collection/element/`, `scenes/c1.rs` |
| Nodes | `browser/selection`, `browser/scrolling`, `browser/view-modes/position-restore` |
| Branch | `feat/<issue>-c1e-collection-element` |

## Goal

The Rune element under all three views: virtual rows over an `EntrySource`, drawn from a
`Selection`, with a seam where the controllers plug in.

## Scope

- Virtual rows (`.virtualize` on a growth stack inside a scroll viewport) over the W1
  `EntrySource` and a `CollectionGeometry`. Takes state by accessor.
- **Row slots**, filled by the host or the view: icon or thumbnail (C25), label or rename
  editor (C5), a drop-target highlight (W1 `DropZone`), a drag source (W1 `DragPayload`
  intent), and trailing cells (C4).
- **Controller seam:** one place where pointer, keyboard and marquee events are routed to
  controllers, and their selection operations are applied. C1b–C1d each plug in through a
  file of their own, so they never edit this WP's files. Land it with no controllers and a
  plain click-to-select for the gallery.
- **Accessibility:** a listbox or grid role, `active_descendant` for the cursor row,
  `position_in_set` and `size_of_set`, and announcing selection changes from C1a's change
  notifications.
- **Position restore:** apply `RestoreMemory` with `scroll_anchor` and `Tx::keep_on_screen`.
  A refresh keeps the cursor row on screen.
- Motion keys per row for C23.
- **Performance:** 100,000 entries scroll at 60 Hz with no view commit per frame. Use
  rune-kit's Table perf test as the bar and the template.

## Tests

- Scripts: click-select, a refresh keeps the cursor and scroll, and restore after
  navigating away and back.
- A11y golden: listbox and options, the active descendant and the set position.
- A perf test at 100k entries (`--profile perf`, ignored by default), plus a scripted scroll
  under `RUNE_COMMIT_BUDGET_MS=8`.
- Scenes: empty, 10 rows, 100k rows, a selection, a drop hover and a rename open. All in light
  and dark.

## Done when

The 100k perf bar is met (part of M2), and C2, C3 and C4's owners have reviewed the slot and
seam API.
