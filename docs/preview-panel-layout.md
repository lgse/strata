# Preview panel and column layout

This page is the contract for how the quick preview panel shares the window with
the file views, in particular with Miller columns, and for how those columns
resize and move. Changes to the preview split, the column scroller, column
resizing or animation, or selection mirroring must keep every rule below or
update this page and the behavioral coverage in the same pull request. Verify
geometry manually with captures; do not add layout assertions.

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
should position the divider except the user's drag and `animate_reveal`, which
slides an unreserved drawer open or closed with the pane pinned at its resting
width. A slot that Columns reserves never slides: content appears in it and
leaves it in place.

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
| Column maximum | the visible browser area: the preview split minus its divider and the sidebar at full width (even while railed), never below the standard width; bounds drags, autofit, new columns, text-size changes, and widths restored from settings, but a window resize does not narrow open columns |
| Columns preview minimum | one standard column, 300 px (`COLUMN_WIDTH`) |
| Icons and List preview minimum | 240 px (`MIN_WIDTH`) |
| Narrow-window hide threshold | 240 px (`MIN_SPLIT_PREVIEW_WIDTH`) |
| Manual width minimum | one standard column, 300 px |
| Icons and List automatic maximum | 3000 px (`MAX_WIDTH`) |
| Image, 3D model, video, PDF, and rendered document content maximum | 1280 px (`media_layout::MAX_CONTENT_WIDTH`), centered in wider previews; PDF zoom scales from it |
| Sidebar rail hysteresis | 24 px (`RAIL_RELEASE_MARGIN`) |
| Peek sliver of earlier columns | never reserved; whatever the focused column and preview leave |

- **Automatic width in Columns** fills the free space right of the navigated
  columns, clamped between the preview minimum (or a dragged width, which
  raises it) and the space that keeps the focused column visible. Trailing columns do not reduce
  it: the preview starts after the focused column, and a pointer preview of a
  file in a parent column closes deeper columns first, like the keyboard mirror.
- **Automatic width in Icons and List** is 90% of half the content width,
  clamped to the minimum and maximum.
- **A boundary between two columns** resizes the left column from 6 px either
  side of its line, the same 6 px its scrollbar is inset by, so the gap between
  the scrollbar and the line is never dead. This holds while that column is
  partly scrolled out of view: its peek strip gives way to the edge.
- **Column width** follows the pointer while a column edge is dragged. When
  the drag ends, the window's other open columns ease to the same width.
  Double-clicking an edge fits that column alone to its widest entry, never
  narrower than the standard width. Neither makes a column wider than the
  visible browser area, so its edge and header actions stay on screen and the
  preview yields instead. Either way the width is saved for new columns; see
  [Preferences](preferences.md).
- **The column–preview boundary** splits by side. The last 6 px inside the
  column that meets the preview resize that column, except its last pixel,
  where the divider's 1 px line is drawn. That line and the first 6 px inside
  the preview, the divider's grip, form the preview side. Hovering either side
  lights the line; the preview side also tints the grip's strip, so the two
  sides are told apart, and crossing between the line and the grip keeps its
  caption. Resting
  the pointer on either side for a moment shows a caption naming what a drag
  would change, **Column width** or **Preview panel minimum width**; a
  column's caption then follows its edge through the drag, and the preview's
  gives way to the minimum outline. The grip and the divider's resize cursor
  exist only while the preview is shown; while it is hidden there is no width
  to set, so only the column side resizes. The divider itself takes only its
  1 px line (a wide handle), so GTK's usual overhang never covers the
  column's resize edge.
- **Manual width** comes from dragging the divider or moving it with the
  keyboard. In Columns it becomes the session's minimum: the preview still
  fills the free space beside the focused column when there is more room, so
  no gap opens between them, and gives way to the columns only down to that
  width. A divider drag outlines the minimum being set, captioned **Preview
  panel minimum width**, until the drag ends; dragging narrower than the space
  the preview fills leaves the panel itself in place. In Icons and List it is
  the preview's width. It is window-local and session-local: it survives
  closing, reopening, and folder changes, and is forgotten when the window
  closes. A manual width can go down to one column but never below the hide
  threshold.
- **Narrow windows**: the file view has priority. The preview shrinks to its
  minimum, then its content hides when less than 240 px would remain beside
  the focused column. In Columns the reserved slot itself never disappears: it
  keeps whatever remains beside the focused column, down to zero, so the
  columns keep their offset at every width. A hidden preview pauses media and
  defers loading; widening the window restores it with the same selection and
  manual width. The sidebar
  collapses to a rail while a preview is present and space is short, with 24 px
  of hysteresis so it does not flicker at the threshold. One rule decides the
  rail: the preview's rule while a preview, placeholder, or reserved Columns
  slot is present, and the window rule (the full sidebar beside one column)
  otherwise. The header toggle, the preview tick, and the chooser all ask it, so
  showing the sidebar opens straight into the rail instead of expanding and
  snapping back. A change in fit compacts an expanded sidebar once; the header
  toggle's steps or the saved compact preference override it either way (see
  [Sidebar compact](preferences.md)). A divider position
  that a narrow window pinned is never recorded as the user's sidebar width:
  the preview is sized against the width the user chose, and the sidebar
  returns to it as soon as the content has room again.
- **Space-constrained priority**: the focused column is never moved to make a
  trailing child column fit. When the lent slot is narrower than the child, the
  child is clipped at the right edge instead, and revealing a column beyond the
  active one reveals the active column.

## The right pane in Columns

1. **Reservation.** Columns reserves the slot from startup. **Space**, **i**,
   **Esc**, and the close button dismiss the content without reclaiming the
   space. **Appearance → Preview panel** off releases it; any explicit preview
   reserves it again. The reservation is window-local, is not a saved
   preference, does not enable automatic previews by itself, and never yields
   to a narrow window (the slot just gets as narrow as the space allows).
2. **A focused folder hands the right pane to its child column.** The drawer
   hides its pane (no placeholder) and the empty slot lends the child column
   exactly its width. The columns scroller and its content grow by the same
   amount, so the scroll offset is unchanged and the focused column does not
   move. The same applies to deeper columns left open after **Left**. A folder
   that a click is opening is the exception: from the press until its column
   takes focus it lends nothing, and the columns beyond it borrow nothing, so
   the strip does not shift and shift back. Switching to a sibling folder swaps
   its column in place: the new column fades in where the old one stood, and
   any deeper columns of the old branch shrink away, so the strip slides once
   if it must move at all.
3. **A focused file takes the right pane back.** The mirror closes the child
   column, the slot grows by the same amount, and the preview fills it. The
   focused column still does not move. Deleting the focused entry mirrors and
   reveals whatever takes its place, folder or file, even when the pointer
   started the deletion.
4. **Files with no preview and empty selections** keep the "No preview for this
   selection" placeholder in Columns and Icons. List hides the drawer instead.
5. **When the focused column does move.** Only when the user descends or
   ascends with **Right**, **Left**, or **Enter**, or clicks a folder into its
   column, and that column does not fit beside the reserved slot, or when the
   user scrolls or resizes. Those
   scrolls reveal only as much as needed.
6. **Priority: focused column, then preview, then peek.** The focused column
   is always fully visible. The preview takes what remains beside it, down to
   its hide threshold. A sliver of earlier columns is whatever space is left
   after those two; nothing is ever reserved for it, so it is the same for
   every selection until the window, the sidebar, or the focused column's
   width changes. `ColumnSpan::reveal_target` in
   `src/ui/browser/columns/reveal.rs` never moves a fully visible column, or
   one wider than the viewport that already fills it, and otherwise applies
   one rule: align the end of the strip (the focused column
   plus the trailing columns that fit beside it) with the right pane. Clipped
   columns show a "Reveal X column" button over whatever part of them is
   visible. The breadcrumbs, the sidebar, and **Left** remain the reliable
   routes to earlier columns.

## Column motion

Column changes take 220 ms (`COLUMN_TRANSITION`) and the drawer's slide takes
260 ms (`TRANSITION`). With reduced motion or GTK animations off, every change
below is immediate.

| Change | Motion |
| --- | --- |
| A column opens | It slides in a short way from the left and fades in. |
| A sibling folder replaces the open one | The new column only fades in, where the old one stood. The old column goes at once, and deeper columns of the old branch shrink away. |
| A column closes with nothing replacing it | It narrows and fades out together, slowly at first, so the columns beside it reflow instead of snapping. While it leaves it takes no clicks, hovers, or drops. |
| Another location opens, or the strip is rebuilt | The columns are replaced at once, with no exit animation. |
| A column edge is dragged | That column follows the pointer every frame. The other open columns ease to its width when the drag ends. |
| A column edge is double-clicked | The column eases to its autofit width, up to the visible browser area. |
| The unreserved drawer opens or closes | The divider slides. Closing takes effect at once, so input returns to the files and only the slide waits. Reopening during the slide turns the drawer back from where it is. A Columns reservation never slides. |

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

## Pointer presses

In every view and in the file chooser, a plain click on blank chrome or on a
disabled control (such as **Next audio file** on the last track) in a preview
that is showing something, including its placeholder, never changes the
selection, so the preview keeps showing it. The empty reserved slot in Columns
shows nothing, so a click there clears the selection like the space beside the
columns. A drag from blank preview chrome into the listing still starts a
marquee. A drag from the preview
header's file icon and name, including the gaps around them, drags the file
instead.

## Verification owners

| Rule | Owner |
| --- | --- |
| Focused column stays put while mirroring folders and files | Manual check below (wide and narrow windows), with captures |
| Folder hands the right pane to the child column, file takes it back | `tests/qa/scenarios/quick_preview/columns-keyboard-opens-preview.jsonl`, `preview-hides-on-folder.jsonl` |
| Dismissed preview ignores mirroring until reopened | `src/ui/preview/tests.rs::an_explicit_close_blocks_automatic_previews_until_reopened` |
| Opening or reopening a folder by click focuses its column, for one or two clicks | `tests/qa/scenarios/click_modes/clicking-open-folder-focuses-column.jsonl`, `src/app/browser/tests/navigation.rs::activating_an_open_folder_focuses_its_column_without_closing_it` |
| Deleting the focused entry mirrors what takes its place, for any input | `tests/qa/scenarios/entry_management/trashing-open-folder-opens-next.jsonl` |
| Reveals leave a viewport-filling column alone and fully reveal a clipped one that fits | `src/ui/browser/columns/tests.rs::reveal_column_does_not_move_an_already_visible_active_column`, `reveal_shows_a_fitting_column_whole_and_leaves_a_filling_one_alone` |
| Pointer preview closes deeper columns first | `src/app/browser/tests/navigation.rs::previewing_a_file_in_a_parent_column_closes_deeper_columns_before_requesting` |
| Automatic width, minimum, and session manual minimum | `tests/qa/scenarios/quick_preview/column-preview-fills-free-space.jsonl` |
| A column boundary resizes the left column from either side, even when it is clipped | Manual check: the GUI scenario was removed in #1601 after keyboard-navigation timing flakes |
| The column–preview boundary resizes the side it is grabbed from | `tests/qa/scenarios/quick_preview/preview-boundary-resizes-grabbed-side.jsonl` |
| Dragged widths spread to the open columns when the drag ends; autofit is saved | `src/ui/browser/columns/tests.rs::a_finished_edge_drag_resizes_every_open_column`, `double_clicking_an_edge_autofits_the_column_and_saves_its_width` |
| Drags, autofit, and restored widths stay within the visible browser area | `src/ui/chooser/tests/column_widths.rs::miller_autofit_saves_a_width_within_the_visible_browser_area`, `::miller_drag_cannot_exceed_the_visible_browser_area`, `tests/qa/scenarios/view_switching/column-autofit-stays-inside.jsonl` |
| Columns open, swap, and close as described in Column motion | `src/ui/browser/columns/tests.rs::a_resize_during_a_column_entry_does_not_strand_its_animation`, `switching_to_a_sibling_closes_the_old_child_without_an_exit_animation`, `a_closing_column_leaves_after_its_animation_and_takes_no_input`, `close_column_skips_exit_animation_when_animations_disabled` |
| Reopening during the drawer's slide out turns it back | `src/ui/preview/tests.rs::reopening_during_the_slide_out_turns_the_drawer_back` |
| Only a shown preview offers its grip and divider | `src/ui/preview/tests.rs::only_a_docked_preview_offers_its_divider_for_resizing` |
| Narrowing a filled preview outlines the minimum and keeps the panel | `src/ui/preview/tests.rs::keyboard_divider_moves_lower_the_columns_session_minimum_without_moving_the_panel` |
| Reservation survives closing; Appearance releases it | `tests/qa/scenarios/quick_preview/columns-preview-reopens-after-closing.jsonl` |
| Narrow windows shrink then hide the preview | `tests/qa/scenarios/quick_preview/narrow-window-keeps-last-column.jsonl` |
| Peek strips reveal clipped columns | `tests/qa/scenarios/preview_session/peek-click-reveals-column.jsonl` |
| Default overflow appearance | `tests/qa/scenarios/visual_baselines/columns-overflow-baseline.jsonl` |
| Blank clicks and disabled controls in a showing preview keep the selection; the empty reserved slot clears; preview drags still marquee | `tests/qa/scenarios/background_selection/preview-blank-click-keeps-selection.jsonl`, `disabled-transport-click.jsonl`, `beside-last-column-clears.jsonl`, `src/ui/marquee/tests.rs::keeping_origins_clear_only_outside_the_content_they_show`, `::chrome_origin_click_policy_decides_when_a_plain_click_clears` |
| Disabled controls are rejected like enabled ones on every chrome origin | `src/ui/marquee/tests.rs::disabled_controls_are_rejected_like_enabled_ones` |

Scenarios that need several parent columns visible at once opt out of the
reservation by switching **Appearance → Preview panel** off through the UI in
their first steps, without changing saved preferences; preview lifecycle
scenarios and the default overflow baseline keep the reservation. See
[testing](testing.md).

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
5. In a window about two columns wide, open a folder, then click one of its
   siblings in the parent column. The new column fades in where the old one
   was and the strip does not shift. Then click a file in the parent column:
   the folder's column narrows and fades out instead of vanishing.
