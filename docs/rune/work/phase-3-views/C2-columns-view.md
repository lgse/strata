# C2 ColumnsView

| | |
| --- | --- |
| Phase / lane | 3 · Collection |
| Plan | §4 C2, `browser/view-modes/columns` (108 bullets), `operations/drag-and-drop/column-autoscroll` |
| Depends on | C1e, C1b, C1c, C1d, C24, C9 (the reserved slot) |
| Unblocks | X2, M2 |
| Owns | `crates/strata-components/src/columns/`, `scenes/c2.rs` |
| Branch | `feat/<issue>-c2-columns-view` |

## Goal

Miller columns: a horizontal strip of C1e collections, each its own virtual list.

## Scope

- A `scroll_x` strip of columns, built from a path of folder sources (data). Each column has
  its own `Selection`.
- Only one column holds the keyboard cursor. Left and right (and `h` and `l`) move between
  columns, and this view decides that, not C1c.
- A new column selects the folder you came from, and scrolls it into view.
- The strip keeps the active column in view, and reserves C9's preview slot at the end.
- During a drag, the strip auto-scrolls near its edges on a glide (`.../column-autoscroll`).
- Column widths are resizable and held per column depth, as Strata does.

## Tests

Scripts: open a folder adds a column, back selects the folder you came from, left and right
move focus between columns, only one cursor, drag near the edge auto-scrolls, and the preview
slot. A11y golden: each column is a listbox with a name, and the active one is announced.
Perf: a column of 100k entries.

## Done when

The columns bullets pass headless, or are listed as Strata policy in the PR.
