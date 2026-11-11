# C4 ListTable

| | |
| --- | --- |
| Phase / lane | 3 · Collection (first of the views: the simplest geometry) |
| Plan | §4 C4, `browser/view-modes/list`, `.../column-layout`, `browser/view-modes/sorting` |
| Depends on | C1e, C1b, C1c, C1d |
| Unblocks | X2, M2 |
| Owns | `crates/strata-components/src/list_table/`, `scenes/c4.rs`, `strata-kit/src/slots/list.rs` |
| Branch | `feat/<issue>-c4-list-table` |

## Goal

The List view: C1e rows with cells, using rune-kit `TableColumn` sizing.

## Scope

- Columns as data (id, title, accessor to cell text or element, and sizing), using
  `TableColumn` resize rules.
- Sortable headings. A click sends a sort intent, and the app sorts
  (`browser/view-modes/sorting`). The heading shows the direction.
- Reorder by dragging headings (an in-window `on_drag`).
- A visibility popover listing columns with checkboxes.
- Sticky file-type group headers, through C1f's `Groups`, set with `CollectionState::set_groups` before `enter` or `sync`: header lines
  in the line model, and one stuck copy the element draws over the viewport (C1f decision; one
  virtual row per section would defeat virtualization for large groups). C4 supplies the groups
  and owns the "sticky header while scrolling" script.
- The column layout is a value type that the host shares across windows and the chooser.
- Fork rune-kit Table only if upstream accepts none of these as issues. Record the issue links.

## Tests

Scripts: sort intent, resize, reorder headings, toggle visibility, sticky header while
scrolling, and selection behaviour through C1. A11y golden: a grid with column headers and
sort state. Perf: 100k rows, sharing C1e's bar.

## Done when

The list and column-layout bullets pass headless. 100k rows meet the perf bar.
