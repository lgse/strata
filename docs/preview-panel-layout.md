# Preview panel and column layout

This page is the contract for how the quick preview panel shares the window with
the file views, and in particular with Miller columns. Changes to the preview
split, the column scroller, or selection mirroring must keep every rule below or
update this page and the owning tests in the same pull request.

## Window split

`src/ui/window/composition/layout.rs` builds the content as nested horizontal
panes:

```
preview split (gtk::Paned)
├─ start: sidebar split (gtk::Paned): [sidebar | browser views]
└─ end:   preview slot (gtk::Box) › revealer › preview pane
```

The browser views are a stack of Columns, Icons, and List. Only the start child
resizes when the window changes size, so the preview keeps its width and the
file views absorb the difference. The file chooser builds the same split and
shares the drawer code in `src/ui/preview.rs` and `src/ui/preview/layout.rs`.

`sync_split` runs once per frame while the drawer is enabled or reserving space.
It owns the slot's visibility, minimum width, and divider position; nothing else
should position the divider except the user's drag.

## Terms

- **Navigated columns**: the columns from the root through the active column,
  the one holding keyboard focus.
- **Trailing columns**: columns beyond the active column. Keyboard mirroring
  opens one for a focused folder, and moving focus left with **Left** keeps the
  deeper columns open.
- **Right pane**: everything to the right of the navigated columns. It shows a
  trailing child column, the preview, or reserved empty space.
- **Reserved**: Columns keeps the preview slot from startup even when nothing is
  previewed, so toggling a preview never moves the columns under the pointer.
- **Enabled**: the drawer follows the selection. **Dismissed**: the user closed
  it on purpose.

## Width rules

Constants live in `src/ui/preview/layout.rs` and `src/ui/preview.rs`.

| Rule | Value |
| --- | --- |
| Standard column width | 300 px (`COLUMN_WIDTH`, user-resizable per column) |
| Columns preview minimum | two standard columns, 600 px (`MIN_COLUMN_MULTIPLIER`) |
| Icons and List preview minimum | 240 px (`MIN_WIDTH`) |
| Narrow-window hide threshold | 240 px (`MIN_SPLIT_PREVIEW_WIDTH`) |
| Manual width minimum | one standard column, 300 px |
| Maximum width | 3000 px (`MAX_WIDTH`) |
| Sidebar rail hysteresis | 24 px (`RAIL_RELEASE_MARGIN`) |
| Peek sliver of the previous column | 48 px (`COLUMN_PEEK_WIDTH`), all or nothing |

- **Automatic width in Columns** fills the free space right of the navigated
  columns, clamped between the preview minimum and the space that keeps the
  focused column (plus its peek strips) visible. Trailing columns do not reduce
  it: the preview starts after the focused column, and a pointer preview of a
  file in a parent column closes deeper columns first, like the keyboard mirror.
- **Automatic width in Icons and List** is 90% of half the content width,
  clamped to the minimum and maximum.
- **Manual width** comes from dragging the divider or moving it with the
  keyboard. It is window-local and session-local: it survives closing,
  reopening, and folder changes, and is forgotten when the window closes. A
  manual width can go down to one column but never below the hide threshold.
- **Narrow windows**: the file view has priority. The preview shrinks to its
  minimum, then hides entirely when less than 240 px would remain beside the
  focused column. A hidden preview pauses media and defers loading; widening the
  window restores it with the same selection and manual width. The sidebar
  collapses to a rail while a preview is present and space is short, with 24 px
  of hysteresis so it does not flicker at the threshold.

## The right pane in Columns

1. **Reservation.** Columns reserves the slot from startup. **Space**, **i**,
   **Esc**, and the close button dismiss the content without reclaiming the
   space. **Appearance → Preview panel** off releases it; any explicit preview
   reserves it again. The reservation is window-local, is not a saved
   preference, and does not enable automatic previews by itself.
2. **A focused folder hands the right pane to its child column.** The drawer
   hides its pane (no placeholder) and the empty slot lends the child column
   exactly its width. The columns scroller and its content grow by the same
   amount, so the scroll offset is unchanged and the focused column does not
   move. The same applies to deeper columns left open after **Left**.
3. **A focused file takes the right pane back.** The mirror closes the child
   column, the slot grows by the same amount, and the preview fills it. The
   focused column still does not move.
4. **Files with no preview and empty selections** keep the "No preview for this
   selection" placeholder in Columns and Icons. List hides the drawer instead.
5. **When the focused column does move.** Only when the user descends or
   ascends with **Right**, **Left**, or **Enter** into a column that does not
   fit beside the reserved slot; when two or more trailing columns are wider
   than the reserved space; when the window is too narrow for a preview; or when
   the user scrolls or resizes. Those scrolls reveal only as much as needed.
6. **Peek slivers are all or nothing.** `ColumnSpan::reveal_target` in
   `src/ui/browser/columns/reveal.rs` never moves a fully visible column. When
   a column must be revealed, one clipped on the right is aligned with the
   right pane, and one clipped on the left is placed with exactly one 48 px
   sliver of the column to its left (`COLUMN_PEEK_WIDTH`), or flush left for
   the root column. The preview's maximum width always leaves that same sliver
   beside the focused column. No allowance depends on the viewport size or on
   whether the panel is open, so the same column lands in the same place
   whatever the panel is doing. Clipped columns show a "Reveal X column"
   button over whatever part of them is visible. The breadcrumbs, the sidebar,
   and **Left** remain the reliable routes to earlier columns.

## Dismissal

| Input | Effect on the drawer |
| --- | --- |
| **Space** / **i** / close button / **Esc** | Closes and marks it dismissed. Columns keeps the slot. |
| **Appearance → Preview panel** off | Closes, marks it dismissed, releases the Columns reservation. |
| **Space** / **i** / **l** / **Right** on a previewable file, context menu Preview | Explicit open: clears dismissal, follows the selection again. |
| Pointer single click with **Single-click file previews** on | Explicit open, same as above. |
| Keyboard mirror onto a previewable file (**Up/Down**, 10xer cursor) | Opens the preview unless it is dismissed. Dismissal lasts until an explicit open, including across folder changes. |

Before the first preview in a window the drawer is not dismissed, so the first
keyboard mirror onto a previewable file opens it when single-click previews are
on.

## Icons and List

Icons reserves the slot while the drawer is enabled so the grid does not reflow
between selections; a folder shows the placeholder. List hides the drawer on a
folder or unsupported file and keeps its horizontal scroll origin. Neither view
has trailing columns, so the right-pane rules above do not apply.

## Owning tests

| Rule | Owner |
| --- | --- |
| Focused column stays put while mirroring folders and files | `tests/e2e/scenarios/test_quick_preview.py::test_columns_keyboard_mirror_keeps_the_focused_column_stationary` |
| Folder hands the right pane to the child column, file takes it back | `test_quick_preview.py::test_columns_keyboard_selection_opens_the_preview`, `test_preview_hides_on_a_folder_and_resumes_when_selection_moves` |
| Dismissed preview ignores mirroring until reopened | `test_quick_preview.py::test_columns_dismissed_preview_ignores_keyboard_mirroring_until_reopened`, `src/ui/preview/tests.rs::an_explicit_close_blocks_automatic_previews_until_reopened` |
| Pointer preview closes deeper columns first | `src/app/browser/tests/navigation.rs::previewing_a_file_in_a_parent_column_closes_deeper_columns_before_requesting` |
| Automatic width, minimum, and session manual width | `test_quick_preview.py::test_column_preview_fills_free_space_and_remembers_a_dragged_session_width` |
| Reservation survives closing; Appearance releases it | `test_quick_preview.py::test_columns_preview_can_reopen_after_closing`, `tests/e2e/scenarios/test_preview_session.py::test_preview_mode_survives_unsupported_selections_and_matches_appearance` |
| Narrow windows shrink then hide the preview | `test_quick_preview.py::test_narrow_window_prioritizes_the_last_column_and_restores_the_latest_preview` |
| Peek strips reveal clipped columns | `test_preview_session.py::test_peek_click_reveals_a_column_without_activating_rows_or_toolbar_actions` |
| Default overflow appearance | `tests/e2e/scenarios/test_visual_baselines.py::test_columns_overflow_baseline` |

Scenarios that need several parent columns visible at once opt out of the
reservation with the `unreserved_columns` fixture; see
[E2E testing](e2e-testing.md).

## Manual check

1. Open Columns with the defaults (single-click previews and mirroring on) and
   press **Right** three or four times into nested folders until the focused
   column sits beside the minimum-width preview slot.
2. Press **Up/Down** across folders and files. The focused column must not
   move; the right pane alternates between the child column and the preview,
   both starting at the focused column's right edge.
3. Press **Space** to close the preview, then **Down** onto another file. The
   preview stays closed. Press **Space** again and it follows the selection.
4. Narrow the window until the preview hides, then widen it: the same file
   returns at the previous width.
