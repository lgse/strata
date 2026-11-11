//! ListTable (C4): the List view, built on the collection element (C1e).
//!
//! Work package: C4, `docs/work/phase-3-views/C4-list-table.md`.
//!
//! [`ListTable`] draws a [`Collection`] as a table: each row's cells come from the host's
//! [`Column`]s, a heading strip above the rows sorts, resizes and reorders them, and a columns
//! menu shows and hides them. The rows, selection, pointer, keyboard, marquee and file-type
//! groups are C1's: the host builds its collection as for any view, and the table adds its
//! parts to it.
//!
//! ```ignore
//! let table = ListTable::new("files", columns, |s: &App| ListParts {
//!     source: &s.listing, collection: &s.coll, table: &s.list, layout: &s.columns,
//!     widths: &s.widths, sort: s.sort_shown(), look: &s.look,
//! }, |s: &mut App| &mut s.list)
//! .on_sort(|s, intent, tx| s.sort_by(intent, tx))       // the app sorts
//! .on_layout(|s, change, _| s.keep_columns(change))     // order and visibility: every window
//! .on_widths(|s, widths, _| s.keep_widths(widths));     // this scope's widths
//!
//! let el = table.build(s, base_collection(s), &mut layer); // the view
//! table.collection(s, base_collection(s)).tick(s, tx);   // the poll
//! ```
//!
//! # What the table adds to the collection
//!
//! [`ListTable::collection`] sets the collection's roles ([`Roles::GRID_ROWS`]), its row
//! height (`list.row.height`), its label (the name cut in the middle, so the extension stays
//! visible; [`ListTable::label`] replaces it), its content (the cells) and its hit regions.
//! [`ListTable::build`] adds the heading strip as the collection's header, and wraps it all in
//! a sideways scroll viewport. The host sets none of those on its own builder; everything else
//! (the clock, controllers' settings, intents, drops, rename, marks) is the host's as usual.
//!
//! # Columns and widths
//!
//! Columns are data ([`Column`]): an id, a title, a default width with bounds, an alignment,
//! whether a heading click sorts, and what the cells show ([`Cell`]). The primary column
//! (Strata's Name) holds the row's icon and label and fills the width the others leave until it
//! is resized by hand; below the columns' minimums the table scrolls sideways (the header and
//! rows together). The sizing follows rune-kit's `TableColumn`, whose fields are private (Rune
//! draft [R-K1]), so its rules are copied rather than its type reused.
//!
//! **Widths follow the text size**, as Strata's `ListColumnLayout::rescale` does: widths are
//! kept and handed to the host in base points (at Strata's default 13 px), and drawn times
//! [`text_scale`], `kit.ty.ui.size` over K2's base size. So a saved width means the same at any
//! text size, and a larger text size widens every column by the same factor.
//!
//! A saved width is bounded before it is drawn ([`Column::bounded`]): one that isn't a finite
//! number takes the column's default, and none is wider than its max, or [`WIDTH_CEILING`].
//!
//! The layout ([`ColumnLayout`]: order and visibility) is one value every window and the
//! chooser share, and [`ColumnWidths`] one per scope (browser, chooser). The table reads both
//! by accessor and hands the host the new value when the person changes one, once: a reorder
//! on its release, a resize on its release, a visibility change on its click. Saving,
//! normalizing a hand-edited file ([`ColumnLayout::normalize`]), sharing and merging are the
//! host's.
//!
//! # Sorting
//!
//! The table never sorts. A click on a sortable heading sends a [`SortIntent`]: the other way
//! on the sorted column, else ascending. The host sorts, and gives the table its sort back
//! ([`SortShown`]); the heading of that column shows the arrow and the accessible sort state.
//! A sort whose field has no column, or whose column is hidden, shows no arrow. Hiding,
//! showing, reordering and resetting columns never send a sort.
//!
//! # Headings
//!
//! - A press and release on a heading is a sort click; past [`REORDER_THRESHOLD`] sideways it
//!   becomes a reorder drag. The headings stay put; the cursor grabs, and a line in the accent
//!   colour marks the drop point on the left edge of the heading the column would precede, or
//!   the last heading's right edge. A drop where the column is, outside the strip, or a drag
//!   Escape cancels, changes nothing. Escape stops there: the file selection stays.
//! - A press within [`RESIZE_EDGE`] of a heading's right edge resizes the column from its drawn
//!   width (no jump), within its bounds; a double press fits it to the widest of its heading and
//!   the cells of the rows on screen, as Strata measures its bound cells.
//! - A right-click on the strip opens the columns menu at the pointer;
//!   [`ListTableState::open_columns_menu`] opens it from elsewhere ("Customize columns…"), and
//!   [`ListTable::columns_button`] is the pane header's Columns button.
//! - A press on the strip's blank space is the collection's: on empty space at the top of the
//!   rows, so a drag from it draws a band (C1e's header rule).
//!
//! # Groups
//!
//! The host sorts its listing so that each file-type group is contiguous, builds the groups
//! with [`groups_from_runs`] and gives them to the collection's state with
//! `CollectionState::set_groups` before `enter` or `sync`. The element draws a header line
//! above each group and keeps the top group's header stuck at the top of the rows (C1f).
//!
//! # What assistive technology hears
//!
//! The collection is a grid named by the host. Its first row is the headings: column headers
//! named by their titles, the sorted one with its direction, the sortable ones pressed to sort.
//! Each entry is a row named by its name, holding a grid cell per shown column, named by its
//! text. The cursor's row is the grid's active descendant (C1e).
//!
//! # Decisions and deviations
//!
//! - **Built on C1e, not on rune-kit's Table** (owner ruling): Table's rows take presses
//!   themselves, so a marquee, C1b's regions and row drags can't reach them, and it has no
//!   reorder, visibility, groups or sideways scroll. The drafts below ask for what C4 needs.
//! - **Headings aren't Tab stops** (owner ruling; a deviation from Strata, whose headings are
//!   GTK buttons): as in rune-kit's Table, the grid is one focus stop. Assistive technology
//!   presses a heading through its press action.
//! - **Fit uses Rune's double press** (owner ruling; a deviation): Rune pairs presses within
//!   its fixed 0.5 s on Linux (S5 U13), where Strata waits 400 ms.
//! - **The sort click after a drag is suppressed by hand**: Rune presses an element whose own
//!   drag was released on it (Rune draft [R-C4-1]). The table doesn't predict that press (Rune
//!   decides it from the heading's drawn, possibly clipped, rect): the release sets a flag,
//!   the press Rune queues with it in the same frame takes it, and the next build clears it,
//!   so a press from assistive technology, which comes in a later frame, always sorts.
//! - **The resize handles are hidden from assistive technology.** They take no steps, so they
//!   would only read as a "Resize …" splitter under every column header. Rune could make
//!   them adjustable (`El::range_value` and `El::on_step`, as rune-kit's pane grip does), an
//!   assistive resize Strata doesn't have; left for a later change.
//! - **C24's scroll bars (C24b).** The rows' bar ([`BarLook::list`]) is the collection's edge
//!   part, inside the sideways scroll, kept to the scroll's visible right edge
//!   (`Bar::follow_across`), so a List narrower than its columns still shows it at the pane's
//!   edge. The sideways scroll's bar (the app-wide one) runs along its bottom. The pointer over
//!   the table shows both; [`ListTableState::set_bar_mode`] shows them always.
//! - **One handler on the sideways scroll.** Rune's `on_scroll_offset` replaces a handler (Rune
//!   draft U-C24b-1), so the table keeps the scroll's one and calls every host follower from it
//!   after its bars ([`ListTable::follow_sideways`]: C22's peek). [`ListTable::build`] returns a
//!   wrapper around the scroll, so a host can't replace the handler by accident.
//! - **Autoscroll** is the host's to install: `AutoscrollPointer` with
//!   [`ListTableState::feed`] and [`ListTable::sideways_target`].
//! - **A view switch mid-drag**: Rune tells an element removed under a drag nothing (Rune draft
//!   [R-C4-2]), so the host calls [`ListTableState::reset`] when the table leaves the view. It
//!   drops a heading drag or resize and lets go of both scroll bars, which would otherwise
//!   come back held and stay on screen after a thumb drag the switch cut off.
//! - **Cells don't know their column** to assistive technology: Rune has no column index or
//!   count (Rune draft [R-C4-3]).
//! - **A resize commits each move**, rebuilding the rows on screen, as rune-kit's Table does
//!   (Rune draft [R-C4-4]).
//! - **The stuck group header and group header lines scroll sideways** with the rows, as GTK's
//!   section headers do.
//!
//! # Left to other work packages and hosts
//!
//! - The columns themselves (Name, Mode, Size, Type, Modified, their defaults and minimums),
//!   their cell text (Mode's permissions, a folder's item count, the MIME description), and
//!   live dates and cut styling: the host's data.
//! - Saving the layout and widths, their scopes, sharing them across windows and the chooser,
//!   and the hidden columns' effect on the sort button's label: the host's.
//! - The sort menu, the direction button, the pane header (Back, Forward, the Columns button's
//!   place, hiding it in 10xer mode), Appearance's "Group by file type" and "Customize
//!   columns…": the host's.
//! - Which groups there are and their order, and the first row on a first visit: the host's.
//!   The first row shown is entry 0, since groups are runs in display order.
//! - The loading skeleton: the host's, from [`ColumnLayout::visible`].
//!
//! # Tests
//!
//! Each Behavior bullet the table implements, and what checks it. Scripts are in
//! `tests/scripts/c4/` (on the gallery page, `crates/strata-gallery/src/scenes/c4.rs`, whose
//! host stands in for Strata's sorting, grouping and saving); "runtime" tests are in
//! `tests/runtime.rs`, unit tests in `tests.rs`, and "gallery" tests in
//! `crates/strata-gallery/tests/c4.rs`. A check marked *partial* covers only the half named;
//! the rest is the host's or another work package's, as the table says.
//!
//! | Bullet (node) | Check |
//! | --- | --- |
//! | list: Name takes the width left…; below the minimum widths the table scrolls horizontally | unit `the_fill_column_takes…` (the rule); `horizontal-scroll` (a drag at one point, a file drag from Mode's cell before the wheel scrolls sideways, starts a band from blank Name space after it) |
//! | list: Clicking the Name, Size, Type, or Modified heading sorts…, reverses it, and an arrow marks it | `sort-intent` (the arrow's direction is in its tree golden); runtime `assistive_technology_presses_a_heading_to_sort` and the five `an_assistive_press_after_…` (a press after a reorder drag released outside the strip, over a heading's clipped part, on itself, Escaped, or of a heading that doesn't sort) |
//! | list: Clicking the Mode heading does not sort | `sort-mode-heading` |
//! | list: Sorted by Extension, Created, or Unsorted, no heading shows an arrow | `sort-no-column`, *partial*: a sort with no column (Extension) shows none; which fields have columns is the host's |
//! | list: Dragging a heading's right edge resizes… without an initial jump | runtime `a_resize_follows_the_pointer…` (the width mid-drag); `resize-follow` (one drag lands at the drawn width plus the travel) |
//! | list: A drag cannot shrink a column below its minimum… | `resize` (Size stops at 70) |
//! | list: Double-clicking a heading's right edge fits the column to its widest content | runtime `a_double_press_on_the_edge_fits…` |
//! | list: Resized widths are saved…; Name is saved only after it is resized itself | runtime `a_resize_follows…`; `resize`, *partial*: the table hands the host widths with no Name until Name is resized; saving and the scopes are the host's |
//! | column-layout: A List pane header has a Columns menu button with the tooltip "Columns" | `columns-button` (its name; the tooltip is its description in the tree golden) |
//! | view-modes: The List view's Columns menu button is named "Columns" | `columns-button` |
//! | column-layout: The popover is headed COLUMNS and lists check items…, a separator, then "Reset to default columns" | `columns-popover`, *partial*: the heading and the items; the separator draws in the PNG golden `c4-columns-menu` only, as Rune puts none in the tree |
//! | column-layout: The Name item is checked and insensitive, with the accessible description… | `columns-popover` (checked and disabled; the description is in its tree golden) |
//! | column-layout: Right-clicking the List heading row opens the same popover at the pointer | `heading-right-click` |
//! | column-layout: Appearance has a "Customize columns…" item… opens the Columns popover on the headings | `customize-opener`, *partial*: what the item opens, through [`ListTableState::open_columns_menu`] (a page key stands in for the item); the Appearance item is the host's |
//! | column-layout: Clicking a checked… item hides that column's heading and cells…; again shows them. The check mark follows the saved state | `toggle-hide`, `toggle-show`, `toggle-check` |
//! | column-layout: A hand-edited `list_hidden_columns` that names `name` loads with Name shown | unit `a_saved_layout_drops…` and `a_tables_normalize…`, *partial*: [`ListTable::normalize`]; reading the file is the host's |
//! | column-layout: "Reset to default columns" restores…, shows every column, and removes… | `reset-columns`, *partial*: the layout; removing the saved keys is the host's, on [`LayoutWhy::Reset`] |
//! | column-layout: Pressing a List heading and dragging it horizontally shows a grabbing cursor and a 3 px accent inset line…; headings do not move | runtime `a_heading_drag_grabs…`; PNG golden `c4-reorder` |
//! | column-layout: The line sits on the left edge of the heading the column would precede | runtime `a_heading_drag_grabs…`; unit `a_dragged_heading_drops…` |
//! | column-layout: For a drop at the end, the line sits on the last heading's right edge | runtime `a_heading_drag_grabs…`; unit `a_dragged_heading_drops…` |
//! | column-layout: Releasing over the headings moves the column…; one drag can move it several positions | `reorder`, `reorder-several` |
//! | column-layout: A drop on the column's current position shows no line and changes nothing | runtime `a_drop_on_the_columns_own_place…` |
//! | column-layout: Moving the pointer outside the heading row hides the line, and releasing there leaves the order unchanged | runtime `outside_the_heading_row…` |
//! | column-layout: A cancelled drag, or headings unmapped mid-drag, leaves the order unchanged and clears the line and cursor | runtime `a_heading_drag_grabs…` (ended with Escape) and `a_table_removed_mid_drag…` (unmapped, with the host's [`ListTableState::reset`]) |
//! | column-layout: Pressing Escape mid-drag and then releasing leaves the column order… unchanged, and keeps the file selection | `reorder-escape` |
//! | column-layout: After a drag cancelled with Escape, the next heading drag reorders and saves as usual | `reorder-escape` |
//! | column-layout: Moving the Name heading does not change the sort | `reorder-name` |
//! | column-layout: A drag only starts after 10 px of horizontal movement; a shorter press and release stays a sort click | `reorder-threshold` |
//! | column-layout: A press within 8 px of a heading's right edge resizes the column and never reorders it | `resize-edge` |
//! | column-layout: Dragging Mode 180 px to the right and releasing gives the order Name, Size, Mode, Type, Modified | `reorder`; unit `a_dragged_heading_drops…` |
//! | column-layout: A drag saves `list_column_order` once, on release, and every window and the chooser then update | `reorder`, *partial*: one change, on release; saving and sharing are the host's |
//! | column-layout: On load, duplicate columns are dropped and missing ones are appended in default order | unit `a_saved_layout_drops…`, `a_tables_normalize…` |
//! | column-layout: Every browser window and the portal file chooser share order and visibility, and update live | runtime `two_tables_over_one_layout…`, *partial*: two tables over one layout in one process; across processes is the host's |
//! | column-layout: Hiding, showing, reordering, or resetting columns never changes the sort key, direction, or file order | `toggle-hide`, `reorder`, `reset-columns` (no sort sent; the gallery's host sorts only on a sort) |
//! | column-layout: A hidden sort column stays the active sort… | `hidden-sort-column`, *partial*: the table sends no sort while its column is hidden; folder sorts and view switches are the host's |
//! | column-layout: Showing the hidden sort column again restores its arrow; clicking it then reverses the existing direction | `hidden-sort-column` |
//! | pointer-intent: In List, unused Name-column space and its row padding are blank, even when Name is not the first column; other column cells count as item content | `name-gutter-and-cells` (with Name last: a band from blank Name space, a drag from a cell that carries the selection, then a band from the row's padding); unit `the_name_columns_blank_space…` |
//! | marquee: A drag from blank space in a pane header, List's heading strip… starts a marquee | `heading-strip-marquee`, *partial*: List's heading strip (C1e's header rule) |
//! | marquee: A press on a button… keeps that control's own behavior and starts no marquee | `reorder-outside`, *partial*: List's headings |
//! | view-modes: Click the first of five entries and Ctrl+click the third. Shift+Down in List… selects only the third and fourth | gallery `a_ctrl_click_then_shift_down…`, *partial*: List |
//! | view-modes: Long filenames are middle-ellipsized in… the List Name column | PNG golden `c4-default` (the tree keeps the whole name, so no script can see the cut) |
//! | selection: A plain click on an entry selects only that entry and replaces the previous selection | `cell-click` (on a cell, through C4's regions); `scroll-100k` (100,000 rows) |
//! | keyboard-navigation: In Icons and List, Up and Down move focus and selection…; In List, Right keeps focus on the current entry… | `keyboard-grid` (Down, Down, Up, then Right) |
//! | marquee: A press on a… scrollbar… keeps that control's own behavior and starts no marquee | `bar-press` (a trough press jumps, no band); runtime `a_sideways_scroll_reaches_both_bars…` (a drag on the rows' bar at the pane's visible edge in a table wider than it) |
//! | scrolling: Middle-clicking the empty background or gutter of a scrollable… List listing starts autoscroll | `autoscroll-start` (a row's margin) |
//! | scrolling: Middle-clicking an entry row starts no autoscroll; the row's open-in-new-tab… runs instead | `autoscroll-row` |
//! | scrolling: Moving the pointer away from the anchor scrolls toward it… on both axes the listing can scroll | gallery `autoscroll_in_a_narrow_list_moves_the_rows_and_the_columns` |
//! | scrolling: The vertical scrollbar of an Icons or List listing is 9 px wide…; The List vertical scrollbar has no gaps at its ends | PNG golden `c4-bars` |
//!
//! The sticky group header has no feature-map bullet (Strata's GTK headers don't stick; the
//! plan adds it), and its stuck copy is hidden from assistive technology, so no script can
//! see it: gallery `the_top_groups_header_stays_stuck…` and the PNG golden `c4-grouped-stuck`
//! check it (owner ruling). Text size (owner ruling): unit `a_larger_text_size_widens…` and
//! runtime `at_a_larger_text_size_a_resize_hands_the_host_base_units`. Saved widths are
//! bounded (unit `a_saved_width_that_isnt…`). Perf: `tests/perf.rs`.
//!
//! # Rune and rune-kit drafts (not filed)
//!
//! Issues C4 found at the pinned rev, written up for the owner to file. Each repro runs
//! headless (`rune::Runtime`).
//!
//! - **R-K1: public `TableColumn` sizing.** `TableColumn`'s width, flex, min, max, align and
//!   sortable fields have no getters, and `sized` (which sizes a cell to its column) is
//!   private, so another table can't reuse Table's sizing. Repro: build
//!   `TableColumn::new("a", "A").width(90.0).min(60.0)` and try to read its width or size an
//!   element by it outside rune-kit: only `id()` and `clamp()` are public. Ask: getters and a
//!   public `TableColumn::size(el, override)`.
//! - **R-K2: reorder columns by dragging headings.** Repro: a `Table` of three columns; drag a
//!   header sideways past the second: nothing moves and no callback runs. Ask: an insertion
//!   line while dragging and `Table::on_reorder(from, to)`, the order kept in `TableState`.
//! - **R-K3: column visibility.** Repro: no `TableState` or `TableColumn` call hides a column
//!   and keeps its width; a secondary press on the header reaches no handler. Ask: a hidden set
//!   in `TableState` and `Table::on_header_secondary_press`.
//! - **R-K4: row groups with sticky section headers.** Repro: a `Table` over 10,000 rows in
//!   sections of 1,000 can't put a header row above each section that sticks while it shows.
//!   Ask: sections with sticky headers (with Rune drafts U-C1f-1 and U-C1f-4).
//! - **R-K5: Table's header over the app's own rows.** Repro: an app with its own virtual rows
//!   (its selection model, marquee and row drags) can't put Table's header and column sizing
//!   over them; Table's rows each take presses with `on_press_tx`, so a press handler on an
//!   ancestor never hears a press on a row. Ask: a public header element and the columns'
//!   resolved widths, for app-drawn rows.
//! - **R-K6: sideways overflow.** Repro: a `Table` whose columns' minimums sum to 600 pt in a
//!   400 pt window: the root clips the rightmost columns and nothing scrolls. Ask: the header
//!   and rows scroll sideways together below the minimums.
//! - **R-K7: fit a column to its content.** Repro: double-click a header edge: it resets the
//!   width to the declared one. Ask: an option to fit to the widest heading or cell on screen;
//!   and closures rather than `fn` pointers for `Table::new`'s accessors, so one app can build
//!   several tables over its own state.
//! - **R-C4-1: `on_press` after the element's own drag.** An element with `on_drag` and
//!   `on_press` hears both when a drag past the threshold is released on it
//!   (`runtime/input/pointer.rs`, `pointer_up`: `Press` is queued whenever `capture_inside`).
//!   Repro: an element 300 pt wide with both; press at x 50, move to 100, release there: the
//!   log is `drag down, drag move, drag up, press`. Ask: no press after the drag moved past the
//!   threshold, or a flag on `Drag` saying the press will follow.
//! - **R-C4-2: no word to a drag whose element went away.** When the element an `on_drag`
//!   belongs to leaves the tree mid-drag, its handler never hears `Phase::Up`, so state it set
//!   at the press stays. Repro: a view that drops the element after the first move; release:
//!   the handler heard `down, move` only. Ask: a cancelled `Phase::Up` when the element is
//!   removed, as Escape gives.
//! - **R-C4-3: column index and count for grid cells.** AccessKit has `column_index` and
//!   `column_count`, but no `El` builder sets them, so a screen reader can't say which column a
//!   cell is in. Repro: a `grid` of `row`s of `gridcell`s with `columnheader`s; `--tree` has no
//!   column index anywhere. Ask: `El::column_index(i)` and `El::column_count(n)`.
//! - **R-C4-4: live widths.** A column resize must commit the view on every move to give each
//!   mounted row's cell its new width. Repro: 30 rows of 5 cells, drag a width 100 frames: the
//!   view runs 100 times. Ask: a live width (as `live_offset` is a live position) that cells
//!   follow without a view run.
//!
//! [R-K1]: self#rune-and-rune-kit-drafts-not-filed
//! [R-C4-1]: self#rune-and-rune-kit-drafts-not-filed
//! [R-C4-2]: self#rune-and-rune-kit-drafts-not-filed
//! [R-C4-3]: self#rune-and-rune-kit-drafts-not-filed
//! [R-C4-4]: self#rune-and-rune-kit-drafts-not-filed

#![warn(missing_docs)]

mod column;
mod groups;
mod header;
mod layout;
mod menu;
mod resolve;
mod rows;
mod sort;
mod state;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use rune::{El, ScrollPos, Tx, col};
use strata_kit::look::Look;

pub use column::{Cell, Column, ColumnId, WIDTH_CEILING};
pub use groups::groups_from_runs;
pub use header::{REORDER_THRESHOLD, RESIZE_EDGE, heading_key};
pub use layout::{ColumnLayout, ColumnWidths, DropSite, LayoutChange, LayoutWhy};
pub use rows::cell_key;
pub use sort::{SortDir, SortIntent, SortShown, next_direction};
pub use state::{ListTableState, MenuAt};

use crate::collection::element::{Collection, CollectionState, Roles, RowCx, Slot, viewport_path};
use crate::contract::EntrySource;
use crate::scroll::{BarLook, Target, overlay};
use resolve::{Inputs, Resolved, resolve};

/// What the table reads from the host's state, in one accessor.
///
/// Used by: hosts.
pub struct ListParts<'a> {
    /// The listing, as the collection shows it.
    pub source: &'a dyn EntrySource,
    /// The collection's state.
    pub collection: &'a CollectionState,
    /// The table's own state.
    pub table: &'a ListTableState,
    /// The column order and visibility, shared by every window and the chooser.
    pub layout: &'a ColumnLayout,
    /// The widths given by hand, in this scope.
    pub widths: &'a ColumnWidths,
    /// The host's sort, as the headings show it.
    pub sort: Option<SortShown>,
    /// The look.
    pub look: &'a Look,
}

type PartsFn<S> = Rc<dyn for<'a> Fn(&'a S) -> ListParts<'a>>;
type TableMutFn<S> = Rc<dyn Fn(&mut S) -> &mut ListTableState>;
type VersionFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> u64>;
type SortFn<S> = Rc<dyn Fn(&mut S, SortIntent, &mut Tx)>;
type LayoutFn<S> = Rc<dyn Fn(&mut S, LayoutChange, &mut Tx)>;
type WidthsFn<S> = Rc<dyn Fn(&mut S, ColumnWidths, &mut Tx)>;
type SidewaysFn<S> = Rc<dyn Fn(&S) -> Box<dyn Fn(ScrollPos)>>;

/// What the table was built with, shared by its parts' handlers.
pub(crate) struct Shared<S> {
    key: Rc<str>,
    columns: Rc<[Column<S>]>,
    parts: PartsFn<S>,
    table_mut: TableMutFn<S>,
    cells_version: Option<VersionFn<S>>,
    label: Option<Slot<S>>,
    on_sort: Option<SortFn<S>>,
    on_layout: Option<LayoutFn<S>>,
    on_widths: Option<WidthsFn<S>>,
    sideways: Vec<SidewaysFn<S>>,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            columns: self.columns.clone(),
            parts: self.parts.clone(),
            table_mut: self.table_mut.clone(),
            cells_version: self.cells_version.clone(),
            label: self.label.clone(),
            on_sort: self.on_sort.clone(),
            on_layout: self.on_layout.clone(),
            on_widths: self.on_widths.clone(),
            sideways: self.sideways.clone(),
        }
    }
}

/// The factor widths are drawn at: the interface text size over K2's base size, so columns
/// widen with the text as Strata's do (`ListColumnLayout::rescale`).
///
/// Used by: C4, hosts (to show a width in base points).
pub fn text_scale(look: &Look) -> f32 {
    let f = look.kit.ty.ui.size / strata_kit::typography::BASE_PX;
    if f.is_finite() && f > 0.0 { f } else { 1.0 }
}

/// The List view's table: see the [module docs](self).
///
/// Used by: hosts (Strata's List pane and the chooser's).
pub struct ListTable<S> {
    shared: Shared<S>,
}

impl<S: 'static> ListTable<S> {
    /// A table keyed `key` (its heading strip and menus' keys start there), of `columns` in
    /// their default order, reading the host's state through `parts` and its own state through
    /// `table_mut`.
    pub fn new(
        key: &str,
        columns: impl Into<Rc<[Column<S>]>>,
        parts: impl for<'a> Fn(&'a S) -> ListParts<'a> + 'static,
        table_mut: impl Fn(&mut S) -> &mut ListTableState + 'static,
    ) -> Self {
        Self {
            shared: Shared {
                key: key.into(),
                columns: columns.into(),
                parts: Rc::new(parts),
                table_mut: Rc::new(table_mut),
                cells_version: None,
                label: None,
                on_sort: None,
                on_layout: None,
                on_widths: None,
                sideways: Vec::new(),
            },
        }
    }

    /// A version of what row `rcx`'s cells read beyond its entry (its size, date, type): the
    /// row is built again when it changes. Without one, cells are built again only with the
    /// entry, its state or the layout.
    pub fn cells_version(mut self, f: impl Fn(&S, &RowCx<'_>) -> u64 + 'static) -> Self {
        self.shared.cells_version = Some(Rc::new(f));
        self
    }

    /// The primary column's label (matched text, say), instead of the middle-truncated name.
    pub fn label(mut self, slot: Slot<S>) -> Self {
        self.shared.label = Some(slot);
        self
    }

    /// Hears a sort click: the host sorts.
    pub fn on_sort(mut self, f: impl Fn(&mut S, SortIntent, &mut Tx) + 'static) -> Self {
        self.shared.on_sort = Some(Rc::new(f));
        self
    }

    /// Hears a new column order or visibility: the host keeps it and shares it.
    pub fn on_layout(mut self, f: impl Fn(&mut S, LayoutChange, &mut Tx) + 'static) -> Self {
        self.shared.on_layout = Some(Rc::new(f));
        self
    }

    /// Hears new widths, once per resize or fit: the host keeps them for this scope.
    pub fn on_widths(mut self, f: impl Fn(&mut S, ColumnWidths, &mut Tx) + 'static) -> Self {
        self.shared.on_widths = Some(Rc::new(f));
        self
    }

    /// Hears the sideways scroll's position on every frame it moves, with no view commit, as
    /// `El::on_scroll_offset` would: `f` makes the handler from the state at each build (C22's
    /// `FolderPeek::follow_sideways`). Each call adds one; the table's own scroll bars follow
    /// the scroll too. The element [`ListTable::build`] returns isn't the sideways scroll, so
    /// this is the only way to follow it.
    pub fn follow_sideways<F: Fn(ScrollPos) + 'static>(
        mut self,
        f: impl Fn(&S) -> F + 'static,
    ) -> Self {
        self.shared.sideways.push(Rc::new(move |s: &S| {
            Box::new(f(s)) as Box<dyn Fn(ScrollPos)>
        }));
        self
    }

    /// The sideways scroll as C24's autoscroll moves it: pass it to
    /// `AutoscrollPointer::sideways` so autoscroll moves the table on both axes.
    pub fn sideways_target(&self, s: &S) -> Target {
        Target::horizontal(
            self.scroll_key(),
            (self.shared.parts)(s).table.sideways_feed.clone(),
        )
    }

    /// The sideways scroll's key.
    fn scroll_key(&self) -> String {
        format!("{}-scroll", self.shared.key)
    }

    /// The columns, in their default order.
    pub fn columns(&self) -> &[Column<S>] {
        &self.shared.columns
    }

    /// A saved layout made whole for this table's columns ([`ColumnLayout::normalize`] with
    /// their default order and the primary column): what a host loads `list_column_order` and
    /// `list_hidden_columns` with.
    pub fn normalize<'a>(
        &self,
        order: impl IntoIterator<Item = &'a str>,
        hidden: impl IntoIterator<Item = &'a str>,
    ) -> ColumnLayout {
        let defaults: Vec<ColumnId> = self.shared.columns.iter().map(|c| c.id.clone()).collect();
        let primary = self
            .shared
            .columns
            .iter()
            .find(|c| c.is_primary())
            .map(|c| &c.id);
        ColumnLayout::normalize(&defaults, primary, order, hidden)
    }

    /// The default layout: every column, in the order given, shown.
    pub fn default_layout(&self) -> ColumnLayout {
        ColumnLayout::new(self.shared.columns.iter().map(|c| c.id.clone()))
    }

    /// The columns as `s` lays them out now.
    fn resolved(&self, s: &S) -> Rc<Resolved> {
        let parts = (self.shared.parts)(s);
        let look = parts.look;
        let lead = strata_kit::slots::collection::row_inset(look) + look.space.xs;
        let live = parts.table.resize.as_ref().map(|r| (&r.column, r.live));
        Rc::new(resolve(
            &self.shared.columns,
            &Inputs {
                layout: parts.layout,
                widths: parts.widths,
                live,
                viewport: parts.table.width,
                lead,
                scale: text_scale(look),
            },
        ))
    }

    /// `base` with the table's parts: its roles, row height, label, cells and hit regions. Use
    /// it for the collection's `tick` and `geometry` too.
    pub fn collection(&self, s: &S, base: Collection<S>) -> Collection<S> {
        let shared = Rc::new(self.shared.clone());
        let resolved = self.resolved(s);
        self.collection_with(s, base, &shared, &resolved)
    }

    /// [`ListTable::collection`] with the columns as this build laid them out.
    fn collection_with(
        &self,
        s: &S,
        base: Collection<S>,
        shared: &Rc<Shared<S>>,
        resolved: &Rc<Resolved>,
    ) -> Collection<S> {
        let region_shared = shared.clone();
        let region_resolved = resolved.clone();
        let table = (self.shared.parts)(s).table;
        let (bar, feed) = (table.bar.clone(), table.feed.clone());
        let edge_bar = bar.clone();
        let (bar_key, target) = (
            format!("{}-bar", self.shared.key),
            viewport_path(base.scope()),
        );
        let edge_shared = shared.clone();
        base.roles(Roles::GRID_ROWS)
            .on_scroll(move |p| {
                feed.set(p);
                bar.follow(p);
            })
            // Inside the sideways scroll, kept to its visible edge (`Bar::follow_across`).
            .edge(move |s: &S| {
                let look = BarLook::list((edge_shared.parts)(s).look);
                Some(edge_bar.build::<S>(&bar_key, &target, look))
            })
            .row_height(|_: &S, look: &Look| strata_kit::slots::list::row_height(look))
            .label(self.shared.label.clone().unwrap_or_else(rows::label))
            .content(rows::content(
                shared,
                resolved,
                rows::Fonts::of((self.shared.parts)(s).look),
            ))
            .regions(move |s: &S, index, local| {
                let parts = (region_shared.parts)(s);
                if index >= parts.source.len() {
                    return crate::collection::element::Region::Gutter;
                }
                let name = parts.source.entry(index).name;
                rows::region(&region_resolved, parts.look, name, local)
            })
    }

    /// The table: `base` with its parts and the heading strip, in a viewport that scrolls
    /// sideways when the columns are wider than it, with C24's scroll bars: the rows' down the
    /// visible right edge, the sideways scroll's along its bottom. The columns menu mounts
    /// into `layer`.
    ///
    /// What it returns wraps the sideways scroll (keyed `{key}-scroll`) rather than being it:
    /// follow that scroll with [`ListTable::follow_sideways`].
    pub fn build(&self, s: &S, base: Collection<S>, layer: &mut Vec<El>) -> El {
        (self.shared.parts)(s).table.end_frame();
        let shared = Rc::new(self.shared.clone());
        let resolved = self.resolved(s);
        let strip = header::strip(&shared, &resolved, s);
        let strip = menu::headings(&shared, s, strip, layer);
        let header = Rc::new(RefCell::new(Some(strip)));
        let list = self
            .collection_with(s, base, &shared, &resolved)
            .header(move |_: &S| header.borrow_mut().take())
            .build(s);
        let parts = (shared.parts)(s);
        let width = parts.table.width;
        let (bar, sideways_bar) = (parts.table.bar.clone(), parts.table.sideways_bar.clone());
        let sideways_feed = parts.table.sideways_feed.clone();
        let sideways = sideways_bar.build::<S>(
            &format!("{}-sideways-bar", self.shared.key),
            &self.scroll_key(),
            BarLook::app(parts.look).horizontal(),
        );
        // The one handler on the sideways scroll (Rune's `on_scroll_offset` replaces, U-C24b-1):
        // the table's bars, then every host follower.
        let followers: Vec<Box<dyn Fn(ScrollPos)>> = shared.sideways.iter().map(|f| f(s)).collect();
        let (follow_bar, follow_sideways) = (bar.clone(), sideways_bar.clone());
        let layout_shared = shared.clone();
        let scroll = col()
            .key(self.scroll_key())
            .w_full()
            .h_full()
            .min_h(0.0)
            .scroll_x()
            .on_scroll_offset(move |p: ScrollPos| {
                sideways_feed.set(p);
                follow_sideways.follow(p);
                follow_bar.follow_across(p);
                for f in &followers {
                    f(p);
                }
            })
            // Columns land at their widths at once, as Strata's do: a width that sprang would
            // put cells where the hit regions don't, and a press mid-spring would miss.
            .instant_changes()
            .on_layout_tx(move |s: &mut S, r, tx: &mut Tx| {
                let st = (layout_shared.table_mut)(s);
                if st.width == r[2] {
                    tx.live_only();
                } else {
                    st.width = r[2];
                }
            })
            .child(
                col()
                    .w(resolved.row_width().max(width))
                    .h_full()
                    .child(list),
            );
        // The sideways bar over the scroll, along its bottom; the pointer over the table shows
        // both.
        bar.area(sideways_bar.area(overlay(scroll, [sideways])))
    }

    /// The Columns button for the pane header, with the columns menu under it (mounted into
    /// `layer`). It is named and helped "Columns"; the host places it, and hides it in 10xer
    /// mode.
    pub fn columns_button(&self, s: &S, layer: &mut Vec<El>) -> El {
        menu::button(&Rc::new(self.shared.clone()), s, layer)
    }

    /// The width the column `id` would fit its heading and the cells on screen at, in base
    /// points: what a double press on its edge sets.
    pub fn fit_width(&self, s: &S, id: &ColumnId) -> Option<f32> {
        let def = self.shared.columns.iter().position(|c| c.id == *id)?;
        Some(header::fit(&self.shared, &self.resolved(s), s, def))
    }
}
