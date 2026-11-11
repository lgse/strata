# 37. rune-kit `Table`: reusable column sizing, reorder, visibility, sideways overflow, fit

| | |
| --- | --- |
| Kind | New issue (related #331) |
| Drafts | R-K1 to R-K7 (C4), U-C11b-2 |
| Affects | C4 (list table, built on its own collection instead of `Table`), C11b (sheet grid) |
| Retires | C4's and C11b's own tables styled as rune-kit's |
| Checked | Re-checked at `2a44156` by source: `TableColumn` still has setters only (`rune-kit/src/table.rs:95–127`) plus `id()` and `clamp()` (`:129`, `:133`), and `sized` is still private (`:662`). Rows still take `on_press_tx` (`:573`), a header edge's double press still resets (`:796`), and `Table::new` still takes `fn` accessors (`:488`). The only change since the pin, `0b07bc85`, moves the resize drag onto a `Grip` with no new API. In the gallery, `mise run test list_table::tests::runtime` passed (15 tests, C4's own table), and C4's and C11b's docs still list R-K1 to R-K7 and U-C11b-2 (`list_table/mod.rs:222–245`, `sheet_grid/mod.rs:221`). Earlier: by source at `5d421fd` and `04b38ae`. No program needed. |
| Status | Ready to file (re-verified at 2a44156, 2026-11-10; no duplicate as of 2026-11-10) |

Paste from here.

---

**Title:** rune-kit `Table`: reusable column sizing, column reorder and visibility, sideways overflow, fit to content, and a header over app rows

Context: Strata (lgse/strata), a file manager on Rune: its list view needs Table's look over its own
rows (its own selection model, marquee and row drags), and a spreadsheet preview needs ranges and
multi-key sorts.

**What's missing** (`5d421fd`, unchanged at `2a44156`):
1. `TableColumn`'s width, flex, min, max, align and sortable have no getters, and `sized` (which
   sizes a cell to its column, `table.rs:662`) is private.
2. No column reorder by dragging headings, and no column visibility (a hidden set that keeps
   widths); a secondary press on the header reaches no handler.
3. No row groups with sticky section headers (see (link to 45)).
4. Table's rows take presses with `on_press_tx`, so an app can't put Table's header and sizing
   over rows it draws itself.
5. Columns whose minimums exceed the width are clipped; nothing scrolls sideways.
6. A double-click on a header edge resets to the declared width; there's no fit to content. The
   accessors are `fn` pointers, so one app can't build several tables over its own state.
7. No cell ranges, no sort by several keys with the later keys shown, no row-header column that
   stays put sideways.

**Ask.** Getters and a public `TableColumn::size(el, override)`; `Table::on_reorder(from, to)`
with the order and a hidden set in `TableState`, and `on_header_secondary_press`; a public
header element and the resolved widths for app-drawn rows; sideways scrolling below the
minimums; fit to the widest heading or cell on screen; closures for the accessors.

Related: #331 (build on each row element).
