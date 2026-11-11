# W1 Component contracts

| | |
| --- | --- |
| Phase / lane | 0 · Groundwork |
| Plan | §3 rules, §4 C1 (rows host drag source, drop highlight, rename, thumbnail), P1 (API shape) |
| Depends on | W0 |
| Unblocks | C1–C8, C14b, C17–C21, X2 |
| Owns | `crates/strata-components/src/contract/` |
| Branch | `feat/<issue>-w1-component-contracts` |

## Goal

Publish the small shared types that let the collection, chrome and DnD tracks build at the
same time without waiting on each other. Keep the types few, plain and documented. Each one
exists because two or more WPs need it.

## Scope

- **Entries** (`contract::entry`):
  - `EntryId`, an opaque, `Copy` and hashable id that is stable across re-sorts and live
    updates.
  - `EntryKind`: file, folder, symlink and special.
  - An index-based `EntrySource` accessor: `len`, `id_at(index)`, `index_of(id)`, and the
    display fields (name, kind, icon key, optional thumbnail key). It is index-based so that
    100k rows never copy into the view.
  - Size and date fields stay out of the contract. C4 reads its columns through its own
    accessors.
- **Geometry** (`contract::geometry`):
  - A `CollectionGeometry` trait: `rect_of(index)`, `index_at(point)`, `range_in(rect)`,
    `neighbor(index, Direction)` and `page_step()`.
  - A uniform-row implementation, for C1's list and its tests. C3's grid and C2's columns
    implement the trait themselves.
  - Marquee hit-testing over unmounted rows, keyboard moves and the anchor for keyboard
    context menus all go through this trait.
- **Drag and drop intents** (`contract::dnd`):
  - `DragPayload`: entry ids for the in-process fast path, plus the URIs and MIME types to
    offer outside.
  - `DropAction` (copy, move, link, ask), and the Ctrl and Shift rule that picks one
    (`operations/drag-and-drop/copy-or-move`).
  - `DropOffer`: the MIME types the drag offers, and its data once dropped.
  - `DropZone`: a component's target id, plus the callbacks for hover, move, drop and
    reject.
  - The shapes mirror Rune's `drag_out`, `on_drop_files` and `on_drop_hover`, and P1's API in
    plan §4. Components drag inside the window with Rune's `on_drag`, and X2 maps these
    types onto P1.
- **Matched text** (`contract::matched`): a label that draws matched character ranges with a
  Kit accent. C6, C17, C18, C19 and C20c all use it. It takes the ranges as data. Each caller
  does its own matching, or uses rune-kit's fuzzy match.
- Doc comments on every public item. A small gallery scene for the matched-text label.

## Out of scope

Behaviour. No selection, layout or platform code. Do not add a type that only one
component uses.

## Tests

- Unit tests for the copy-or-move rule and for the uniform geometry (`range_in` past the
  mounted window, `neighbor` at the edges).
- An a11y golden for the matched-text label, which must read as plain text with no markup.

## Done when

Rustdoc for every public type, the tests above, and a note in each type's docs naming the WPs
that use it. C1a, C6 and C8 owners review the PR before it merges.
