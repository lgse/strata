//! SheetGrid (C11b): Strata's spreadsheet preview, for CSV, TSV, XLS, XLSX and ODS, drawn
//! natively with sheet tabs, row numbers and column letters.
//!
//! No work package doc: split from C11 (`docs/work/phase-2-composites/C11-document-view.md`,
//! its spreadsheet half), scope from its status row. Feature map node
//! `preview/preview-panel/documents` ("Tables and workbooks"), against Strata's
//! `src/services/table.rs` and `src/ui/table_view*` at `upstream/main`
//! 491e6ba9735e821f9f49910ffd2582076196865c.
//!
//! ```ignore
//! // State: grid: SheetGridState, drawer: DrawerState.
//! s.grid.set_single(Sheet::new("people.csv", rows));            // a CSV or TSV file
//! s.grid.set_workbook(names); s.grid.set_sheet(0, first);       // a workbook, sheet by sheet
//! let view = SheetGrid::new(|s: &App| &s.grid, |s: &mut App| &mut s.grid)
//!     .on_intent(|s: &mut App, SheetIntent::Load(i), tx| s.read_sheet(i))
//!     .build(s, &s.look, cx);
//! PreviewDrawer::new(..).hosted(&s.grid, |s| &mut s.grid).content(view).build(..);
//! // poll: changed |= s.grid.tick(now, &s.look, tx);   on closing the preview: s.grid.stop();
//! ```
//!
//! Rune 2a44156: **partly**. rune-kit's `Table` covers 100,000 virtual rows, a fixed
//! header with a sort caret and resize handles, and the `grid`, `row`, `columnheader` and
//! `gridcell` roles; but it selects whole rows only, sorts by one key, can't scroll sideways
//! (C4's draft R-K6), has no row-header column, and its cells take no presses (R-K5, Rune
//! #331). Rune core covers `virtual_rows`, `scroll_xy`, `sticky_left`, `live_offset`, the
//! sorted state and set positions on virtual rows; rune-kit's `TabStrip` covers the sheet
//! tabs. Nothing covers cell ranges, sorting by several keys, or a grid's row and column
//! index and count. Since 5d421fd rune-kit's `Table` gained only its column-resize grip
//! (0b07bc85), which is crate-private.
//!
//! # The data and what stays in Strata
//!
//! Strata's sandboxed reader turns a file into [`Sheet`]s: plain text cells (calamine's
//! string conversion; no cell types, formulas or merged cells), rows possibly ragged, the
//! first row the titles unless it is empty, cut at its 100,000-value, 4 MiB and 256-column
//! budget (`truncated`). A title shows Strata's 128-character cut, marked with "…".
//! A CSV or TSV is one sheet ([`SheetGridState::set_single`]); a workbook gives its sheet
//! names first ([`SheetGridState::set_workbook`]), then each sheet's data
//! ([`SheetGridState::set_sheet`]) or why it couldn't be read
//! ([`SheetGridState::set_sheet_failed`]): the first one Strata reads itself, the others when
//! the person picks their tab ([`SheetIntent::Load`]). Reading, budgets, the Source view and
//! opening the file are Strata's.
//!
//! # How it draws
//!
//! - **One grid per sheet**, its own element styled as rune-kit's Table (D1): the header (a
//!   corner, then each column's letters over its title), then one virtual row per body row
//!   in a `scroll_xy` viewport (`{key}-s{i}-scroll`), each row a row number held at the left
//!   edge (`sticky_left`) and its cells. The header's titles follow the rows sideways through
//!   `live_offset`, set from the viewport's `on_scroll_offset` (probe P1: in the same frame).
//!   Rows are one fixed height, so a point's cell is arithmetic, wherever Rune has mounted.
//! - **Rows are keyed by display position** (`{key}-r{i}`, built once per sheet), a row's number
//!   `{key}-r{i}-n` and its cells `{key}-r{i}-c{j}`. A re-sort changes no key, so Rune neither
//!   re-reads nor copies 100,000 of them (Rune #203), and `virtual_rows_version` holds for the
//!   sheet, so no commit calls `key` per row.
//! - **Column windowing**: on a wide sheet only the columns that show and half a viewport's
//!   width either side are built (the rows report every frame of a scroll through
//!   `on_layout_tx`, which rebuilds only when what shows leaves what's built: about once half a
//!   viewport of sideways travel). A key that moves the cursor builds its column, so the
//!   active descendant names a cell; after a press, a scroll can take the cursor's cell out of
//!   what's built or mounted (sideways or down), and the grid then has no active descendant
//!   until the cursor shows again (Rune drops one that names no node). Assistive technology
//!   hears the built columns, as it hears the mounted rows.
//! - **Row numbers are the sheet's own**: the title row is row 1, so data starts at 2, and a
//!   sorted sheet shows where each row sits in the file.
//! - **Widths** follow Strata's rule ([`widths`]): the first 64 rows measured, each column
//!   72 to 640 points, the spare width shared out while nobody has resized a column. The first
//!   resize keeps every column at its drawn width from then on.
//! - **C24's overlay bars** down the right and along the bottom, over the viewport.
//! - **Sheet tabs** below the grid, rune-kit's `TabStrip` without close buttons or reordering
//!   (probe P5: without a drag state it takes no drag), shown with two or more sheets. Each
//!   sheet keeps its sort, widths, cursor and scroll position while the preview is open; its
//!   range is dropped when another sheet shows. `TabStrip` leaves revealing the active tab to
//!   the app, as C7's tabs do: each switch (a tab, Ctrl+Page Up or Down, or
//!   [`SheetGridState::set_active`] on its next tick) scrolls it into the strip, to the edge
//!   and under the edge fade (C7's draft U-C7-7).
//! - **A sheet that couldn't be read** ([`SheetGridState::set_sheet_failed`]) shows Strata's
//!   "Preview unavailable" over the reason, in the grid's place, with a Try again button that
//!   asks [`SheetIntent::Load`] again (Strata's failed preview has no button; picking the
//!   sheet's tab again asks nothing). The drawer isn't busy meanwhile.
//!
//! # Sorting
//!
//! As Strata's ([`sort`]): keys per column on its first sort (a finite number, else the text
//! lowercased, else empty; ascending puts numbers first, then text, then empty cells), dense
//! or sparse by how many rows have the column; a stable sort of a row permutation by every
//! sort column most recent first, ties in source order. A click on the primary title flips it,
//! by reversing its groups in linear time; a click on another title makes it the primary,
//! ascending, the others after it. A sheet opens sorted ascending by its first column. A sort
//! change clears the range and scrolls back to the top. Copying reads the permutation, so it
//! follows the displayed order.
//!
//! # Ranges, keys and copy
//!
//! A press on a cell puts the cursor there; a drag spans a rectangle of whole cells from it
//! (D2), autoscrolling near the edges ([`SheetGridState::tick`], C1d's edge speeds and glide),
//! and Shift with a press grows it (D6). A range exists only while it covers more than the
//! cursor's cell. Keys are in [`keys`]. Ctrl+C copies the range as tab-separated rows in the
//! displayed order, or with no range the title row and every loaded row; cells copy whole.
//!
//! # Accessibility
//!
//! The grid is a focusable `grid`, named by its sheet (by "Table" for a file without sheet
//! names); its cursor cell is the active descendant (probe P3). The header is a `row`: the
//! corner a `columnheader` "Row", then each column's `columnheader`, named by its title or
//! "Column C", described by its letters and, for a later sort key, its place and direction;
//! the primary sort column carries the sorted state. Each body row is a `row` (Rune gives its
//! place among all the rows) holding a `rowheader` (its number) and its `gridcell`s, those in
//! the range selected; a cell shown shortened carries Strata's description. A divider is a
//! `separator` whose value is its column's width. The tabs are a `tablist` "Sheets". A sheet
//! that couldn't be read has no rows: its place is a `group` named by the sheet.
//!
//! # Deviations
//!
//! - **D1** (OWNER) An own grid element rather than rune-kit's `Table` (C4's precedent), for
//!   the reasons in the coverage line; it reuses `TableStyle`, `SortDir` and `TabStrip`.
//! - **D2** (OWNER) Ranges are rectangles of whole cells. Strata's are text ranges in reading
//!   order with character offsets in the end cells, and a cell's own text can be selected.
//!   So two bullets are not met: "With a non-Latin layout such as Russian active, Ctrl+A in a
//!   focused table cell selects that cell's text" and "…Ctrl+C in a focused table cell copies
//!   its own selection": there is no text selection inside a cell. (Ctrl+A and Ctrl+C go by
//!   Rune's layout-aware command keys, so on any layout Ctrl+A clears a range and Ctrl+C
//!   copies.)
//! - **D3** Row numbers and column letters: new against Strata, as the plan asks.
//! - **D4** Sheet tabs: new against Strata, which previews the first worksheet only (a known
//!   gap). Strata's sandbox has to send the sheet names and read a sheet on request.
//! - **D5** Rows are one line high: a cell shows its first line, and "…" when it has more or
//!   runs past 512 bytes (GTK's rows grow to a multi-line cell).
//! - **D6** New against Strata: a cell cursor that the arrows, Home, End, Page Up and Page
//!   Down move (Ctrl+Home and Ctrl+End to the ends), Shift with them or with a press growing
//!   the range. Strata's arrows only clear the range, as these do.
//! - **D7** New against Strata: a double click on a divider puts that column back to its
//!   fitted width (rune-kit's Table does so).
//! - **D8** New against Strata: Ctrl+Page Up and Ctrl+Page Down show the previous and next
//!   sheet.
//! - **D9** C24's overlay bars, as C11 (its D5).
//! - **D10** Tables inside documents (Markdown, HTML, DOCX, ODT) stay C11's static tables:
//!   wiring this grid into them is a follow-up.
//! - **D11** Rune doesn't end a drag whose element goes (probe P4): the host calls
//!   [`SheetGridState::stop`] when it closes the preview, which ends the range drag and its
//!   autoscroll ("Closing the preview during a table cell drag ends the drag and its
//!   autoscroll"); a glide under way stops dead on the next tick.
//! - **D12** (OWNER) Opening a 100,000-row sheet takes one commit of about 37 ms: Rune reads and
//!   copies every row's key the first time a list shows (Rune #203); later commits skip it.
//! - **D13** (OWNER) On a 256-column sheet the wheel runs p95 5.1 ms down and 4.7 ms sideways,
//!   over the plan's 4 ms (none over 8 ms), with column windowing. In debug builds a fast
//!   sideways scroll (1,500 points a frame) leaves the built columns on most frames, and Rune
//!   warns that the rows' `on_layout_tx` "committed the view on 6 of 7 frames of a scroll":
//!   each of those commits is the rebuild windowing needs, not a missed `Tx::live_only`.
//! - **D14** Rows keyed by display position, not by row: Rune and assistive technology see a
//!   re-sort as the same rows with new content (the range is cleared by it anyway).
//! - **D15** (OWNER) A title click on 100,000 rows sorts inside the press handler: 19.4 ms for a
//!   text or a number column, 4.2 ms for a flip (`tests/perf.rs`, the click's frame), over the
//!   8 ms frame. Rune's commit budget doesn't see it, since the time is the handler's, not the
//!   commit's. Sorting off the frame (a worker, the old order shown meanwhile) is the fix if
//!   it's wanted.
//! - **D16** Keys that clear the range are taken when there is one: Escape with any modifiers,
//!   and the arrows, Home and End with Ctrl, Alt or Super (Strata's `ESCAPE_ANY` and its
//!   arrows, Home and End ignoring the shortcut modifiers). Strata clears and passes the key
//!   on, so there Escape with a range also reaches the preview; here it doesn't. Without a
//!   range they pass on. Escape during a range drag ends the drag and clears the range: Rune
//!   takes the key to cancel the drag, so neither the grid nor the preview hears it.
//!
//! # Bullets
//!
//! `preview/preview-panel/documents`, "Tables and workbooks" (and one Scrolling bullet), at the
//! feature map's watermark. Scripts are in `tests/scripts/c11b/`; in-process tests in
//! `strata-gallery/tests/c11b.rs` and this module's `tests.rs`.
//!
//! | Bullet | Checked by |
//! | --- | --- |
//! | CSV and TSV files render as a table whose first row is the column titles… | `first-row-titles` |
//! | XLS, XLSX, and ODS files show their first worksheet as a table with no Source view… | `first-worksheet` (sheet tabs beyond it: D4) |
//! | Tables open sorted ascending by the first column; clicking a column title toggles… | `sort-toggle` |
//! | Clicking a second column title sorts by it… | `secondary-sort`; unit: Strata's QA scenario click by click |
//! | Clicking the primary title again reverses its value groups… | `secondary-sort`; unit: the flip against a full sort |
//! | After a … table sorted by two columns scrolls away and back… | not here: tables inside documents (D10) |
//! | While an 18,000-row, 4-column XLSX preview loads… | Strata's reader; perf: an 18,000-row sheet arriving |
//! | Ascending sort puts numbers first, then case-insensitive text, then empty cells… | `sort-order-types`; unit: both directions |
//! | Changing the sort clears a selected cell range and scrolls the table to the top. | `sort-clears-range`; unit |
//! | A table cell over 512 bytes shows shortened with "…"; copying the table keeps its full text. | `long-cell-cut`; unit: the cut and the copy |
//! | Every column, including the last, resizes by dragging its divider. | `resize-every-column`; unit |
//! | Tables over 200 rows or 512 cells render in full. | `scroll-100k` |
//! | CSV, TSV, and workbook output stops at 100,000 values… | Strata's reader |
//! | A limited table shows "Table preview reached its text, column, or value budget…" | `limited-notice` |
//! | Dragging across cells and pressing Ctrl+C copies the range as tab-separated rows… | `range-drag`; unit: the clipboard |
//! | Closing the preview during a table cell drag ends the drag and its autoscroll. | gallery: `closing_the_preview_during_a_cell_drag…`; unit (D11) |
//! | With focus on the table, Ctrl+C copies every loaded row in the current sort order… | unit: the clipboard |
//! | With focus on the table, Ctrl+Shift+C, Ctrl+Alt+C, and Ctrl+Super+C copy nothing. | unit: the clipboard |
//! | With a cell range selected, Ctrl+A or Ctrl+Shift+A clears the range; Ctrl+Alt+A leaves it. | `ctrl-a-clears`, `ctrl-shift-a-clears`, `ctrl-alt-a-keeps` |
//! | With a cell range selected, Escape, an arrow key, Home, or End also clears it. | `escape-clears`, `arrow-clears`, `home-clears`, `end-clears`; gallery: with Strata's modifiers (D16); unit: Escape during a drag |
//! | With a non-Latin layout such as Russian active, Ctrl+A in a focused table cell… | **not met** (D2) |
//! | With a non-Latin layout active and no cell range selected, Ctrl+C in a focused table cell… | **not met** (D2) |
//! | After smooth scrolling, a click, drag selection, or link hover acts on the row under the pointer. | `click-after-scroll` |
//!
//! # Measured
//!
//! `tests/perf.rs`, perf profile, NVIDIA, the final run: 100,000 rows of four columns hand over
//! (with the first sort) in 8.9 ms and draw a first frame in 36.9 ms (D12); 600 wheel frames
//! at 120 Hz run p50 0.27 ms, p95 1.45 ms, max 1.65 ms, none over 8 ms; Ctrl+End and
//! Ctrl+Home p95 4.8 ms; a cell click p95 0.84 ms; a title click sorts a text column in
//! 19.4 ms, flips it in 4.2 ms and sorts a number column in 19.4 ms; Ctrl+C of every row
//! (2.7 MB) takes 6.5 ms; a range drag held at the bottom autoscrolls at p95 1.33 ms. 390 rows
//! of 256 columns: first frame 17.6 ms, the wheel down p95 5.07 ms (max 6.37) and sideways
//! p95 4.67 ms (max 6.57), none over 8 ms (D13). Before column windowing the same sheet ran
//! p95 184 ms down and 31 ms sideways; before rows were keyed by position, a sort's commit took
//! 30 ms more (Rune re-reading and copying 100,000 changed keys). An 18,000-row sheet arrives
//! in 1.0 ms plus a 6.4 ms frame.
//!
//! # Rune gaps (drafts, not filed)
//!
//! - **U-C11b-1** (draft, not filed) A grid's size and a cell's place: AccessKit has
//!   `row_count`, `column_count`, `row_index` and `column_index`, and Rune sets none, so a
//!   screen reader can't say "row 50,000 of 100,000, column C"; it hears only the mounted
//!   rows' set positions. The set size Rune gives sits on the virtual list's own node, so a list
//!   that is layout to assistive technology (`role("none")`, as here, so the rows are the grid's)
//!   loses it. Repro: a `grid` over `virtual_rows(100_000, ..)` with `gridcell`s;
//!   `Runtime::accessibility_tree` has no counts on the grid and no indices on the cells.
//!   Ask: `El::grid_size(rows, cols)` and `El::grid_place(row, col)`.
//! - **U-C11b-2** (draft, not filed) rune-kit `Table`: cell ranges, sorting by several keys
//!   with the later keys shown, and a row-header column that stays put sideways. With C4's
//!   R-K5 and R-K6 and Rune #331, which ask for the header over app rows and sideways
//!   scrolling. Repro: a `Table` of 4 columns: a drag across cells selects rows, a second
//!   title click drops the first sort, nothing scrolls sideways below the minimums.
//! - **U-C11b-3** (draft, not filed) A wide virtual grid builds every column of every mounted
//!   row: virtual rows are vertical only, and a sticky child makes its subtree draw live, so 256
//!   columns ran p95 31 ms a frame sideways and 184 ms down before this grid windowed its
//!   columns itself from `on_layout_tx`. Repro: `virtual_rows(390, ..)` in `scroll_xy`, each
//!   row a `sticky_left` cell and 256 text cells; wheel sideways. Ask: virtual columns, or a
//!   scroll callback that may change state when a sideways window is crossed. Meanwhile Rune's
//!   debug commit warning on `on_layout_tx` fires on fast sideways scrolls (D13).
//!
//! Also seen: Space doesn't reach `on_key` while a pointer is held (Rune keeps it for its
//! Space-drag pan), so the gallery closes its preview mid-drag on F9.

mod copy;
mod keys;
mod model;
mod range;
mod render;
mod sort;
mod state;
mod widths;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cx, El, GrowthSpec, KeyEvent, ScrollSpec, Tx, col, text};
use rune_kit::shell::{TabItem, TabOrientation, TabOverflow, TabStrip};
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;

use crate::dialog::FOCUS_RING_WIDTH;
use crate::scroll::{BarLook, overlay};

pub use model::{CELL_DISPLAY_BYTES, Sheet, TITLE_DISPLAY_CHARS, column_letters};
pub use range::{CellPos, CellRange};
pub use rune_kit::SortDir;
pub use state::{SheetGridState, SheetIntent, SheetLabels};

use render::{Ctx, Paint, build_row, failure, header, placeholder};

type IntentFn<S> = Rc<dyn Fn(&mut S, SheetIntent, &mut Tx)>;

/// The sheet grid: `SheetGrid::new(get, get_mut) … .build(s, look, cx)`.
pub struct SheetGrid<S> {
    get: fn(&S) -> &SheetGridState,
    get_mut: fn(&mut S) -> &mut SheetGridState,
    intent: Option<IntentFn<S>>,
}

impl<S: 'static> SheetGrid<S> {
    pub fn new(get: fn(&S) -> &SheetGridState, get_mut: fn(&mut S) -> &mut SheetGridState) -> Self {
        SheetGrid {
            get,
            get_mut,
            intent: None,
        }
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, SheetIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let SheetGrid {
            get,
            get_mut,
            intent,
        } = self;
        let st = get(s);
        let key = st.key.clone();
        let mut out = col().key(format!("{key}-box")).w_full().h_full().min_h(0.0);
        if let Some(v) = st.view() {
            out = out.child(grid(s, look, cx, get, get_mut, &intent, v));
        }
        if st.sheet_count() >= 2 {
            out = out.child(tabs(st, look, cx, get_mut, &intent));
        }
        out
    }
}

fn grid<S: 'static>(
    s: &S,
    look: &Look,
    cx: &mut Cx<S>,
    get: fn(&S) -> &SheetGridState,
    get_mut: fn(&mut S) -> &mut SheetGridState,
    intent: &Option<IntentFn<S>>,
    v: &state::SheetView,
) -> El {
    let st = get(s);
    let key = st.key.clone();
    let labels = &st.labels;
    let k = &look.kit;
    let name = if v.name.is_empty() {
        labels.table.clone()
    } else {
        v.name.to_string()
    };
    let on_key = intent.clone();
    let mut root = col()
        .key(key.clone())
        .role("grid")
        .label(name)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .focusable(true)
        // Keyboard focus shows inside the edge: the drawer's slot clips anything outside.
        .focus_ring(FOCUS_RING_WIDTH, -FOCUS_RING_WIDTH, k.c.border_focus)
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            match keys::key_in(get_mut(s), ev, tx) {
                keys::Outcome::Ignored => false,
                keys::Outcome::Taken => true,
                keys::Outcome::Load(i) => {
                    if let Some(f) = &on_key {
                        f(s, SheetIntent::Load(i), tx);
                    }
                    true
                }
            }
        });
    // Focus stays on the grid; the cursor's cell is what a screen reader follows (probe P3).
    if let Some(c) = v.sel.map(|s| s.cursor) {
        root = root.active_descendant(format!("{}-c{}", st.row_key(c.row), c.col));
    }
    if let Some(reason) = &v.failed {
        let retry = intent.clone();
        // No rows to speak of: a group named by its sheet, around the reason and the button.
        return root.role("group").child(failure(
            look,
            &key,
            labels,
            reason,
            move |s: &mut S, tx| {
                if let Some(i) = get_mut(s).retry()
                    && let Some(f) = &retry
                {
                    f(s, SheetIntent::Load(i), tx);
                }
            },
        ));
    }
    let Some(data) = v.data.as_ref() else {
        return root.child(placeholder(look, format!("{key}-loading"), &labels.loading));
    };
    if data.truncated {
        root = root.child(
            text(labels.limited.clone())
                .key(format!("{key}-limited"))
                .wrap()
                .w_full()
                .font(k.ty.text_small)
                .color(k.c.text_muted)
                .px(look.space.md)
                .py(look.space.xs),
        );
    }
    if v.columns() == 0 {
        return root.child(placeholder(look, format!("{key}-empty"), &labels.empty));
    }

    let paint = Paint::from_look(look);
    let scroll_key = st.scroll_key();
    let gutter = paint.gutter(data.rows.len());
    let width = cx.layout_size(&scroll_key).map(|z| z[0]);
    let fitted = fitted_widths(v, &paint, width.map(|w| w - gutter));
    let widths = v.manual.clone().unwrap_or_else(|| fitted.clone());
    let cols = range::Columns::new(gutter, &widths);
    *st.laid.borrow_mut() = cols.clone();
    *st.fitted.borrow_mut() = fitted;
    st.row_h.set(paint.style.row_height);
    let built = st.built_columns(&cols, width.unwrap_or(1000.0));
    let ctx = Rc::new(Ctx {
        get,
        get_mut,
        paint,
        labels: labels.clone(),
        cols,
        built,
    });

    let rows_ctx = ctx.clone();
    // The viewport and the list are layout to assistive technology (`none`), not the buttons
    // their drag handlers would make them: the rows are the grid's own.
    let list = col()
        .key(st.list_key())
        .role("none")
        .w_full()
        .min_w(ctx.cols.width())
        .growth(GrowthSpec::default())
        .virtual_rows(
            v.rows(),
            paint.style.row_height,
            move |s: &S, i| get(s).row_key(i).into(),
            move |s: &S, i| build_row(&rows_ctx, s, i),
        )
        // Keys are by position: one set per sheet and row count, so no commit reads them.
        .virtual_rows_version((st.active, v.rows()))
        // Column windowing: the rows report where they've scrolled to on every frame of a
        // scroll; only leaving the built columns rebuilds the view.
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).follow_columns(r[0]) {
                tx.live_only();
            }
        })
        .on_drag(move |s: &mut S, d: &mut rune::Drag, tx: &mut Tx| {
            let st = get_mut(s);
            let changed = match d.phase {
                rune::Phase::Down => {
                    st.press(d.x, d.y, tx.mods().shift);
                    true
                }
                rune::Phase::Move => st.drag_to(d.x, d.y),
                // Escape while the drag is held: Rune ends it cancelled and the key reaches
                // no handler. Strata's capture-phase key controller clears the range on it.
                rune::Phase::Up if d.cancelled() => {
                    st.release();
                    st.clear_range()
                }
                rune::Phase::Up => {
                    let moved = st.drag_to(d.x, d.y);
                    st.release();
                    moved
                }
            };
            if !changed {
                tx.live_only();
            }
        });
    let (feed, vbar, hbar, strip_x) = (
        st.feed.clone(),
        st.vbar.clone(),
        st.hbar.clone(),
        st.strip_x.clone(),
    );
    let viewport = col()
        .key(scroll_key.clone())
        .role("none")
        .w_full()
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_xy()
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).viewport_laid(r) {
                tx.live_only();
            }
        })
        .on_scroll_offset(move |p| {
            feed.set(p);
            vbar.follow(p);
            hbar.follow(p);
            strip_x.set([-p.offset[0], 0.0]);
        })
        // A press below the last row is on empty space: it clears the range.
        .on_drag(move |s: &mut S, d: &mut rune::Drag, tx: &mut Tx| {
            if d.phase != rune::Phase::Down || !get_mut(s).clear_range() {
                tx.live_only();
            }
        })
        .child(list);
    let vbar = st
        .vbar
        .build::<S>(&format!("{key}-vbar"), &scroll_key, BarLook::app(look));
    let hbar = st.hbar.build::<S>(
        &format!("{key}-hbar"),
        &scroll_key,
        BarLook::app(look).horizontal(),
    );
    let body = st.hbar.area(st.vbar.area(overlay(viewport, [vbar, hbar])));
    root.child(header(&ctx, st, v, &st.strip_x))
        .child(col().w_full().grow(1.0).min_h(0.0).child(body))
}

/// What each column is drawn at while nobody has resized one: what it wants, fitted to
/// `available` once the viewport's width is known.
fn fitted_widths(v: &state::SheetView, paint: &Paint, available: Option<f32>) -> Vec<f32> {
    let size = paint.style.font.size.to_bits();
    let mut wanted = v.wanted.borrow_mut();
    if wanted.as_ref().is_none_or(|(s, _)| *s != size) {
        let data = v.data.as_ref().expect("a laid-out sheet has data");
        let (body, title) = (paint.style.font, paint.style.header_font);
        let w = widths::wanted(data, paint.style.cell_pad, |t, head| {
            rune::text::advance(t, if head { title } else { body })
        });
        *wanted = Some((size, w));
    }
    let want = &wanted.as_ref().expect("measured").1;
    match available {
        Some(a) if a > 0.0 => widths::fit(want, a),
        _ => want.clone(),
    }
}

fn tabs<S: 'static>(
    st: &SheetGridState,
    look: &Look,
    cx: &mut Cx<S>,
    get_mut: fn(&mut S) -> &mut SheetGridState,
    intent: &Option<IntentFn<S>>,
) -> El {
    let key = format!("{}-tabs", st.key);
    let ids: Vec<String> = (0..st.sheet_count())
        .map(|i| format!("{}-sheet{i}", st.key))
        .collect();
    let items = st.sheets.iter().zip(&ids).map(|(v, id)| {
        // No avatar mark: a sheet has no icon of its own (rune-kit draws an empty box).
        TabItem::new(id.as_str(), v.name.to_string())
            .avatar(rune::el().size(0.0, 0.0))
            .closable(false)
    });
    let intent = intent.clone();
    let prefix = format!("{}-sheet", st.key);
    let strip = TabStrip::new(key.clone(), TabOrientation::Horizontal)
        .tabs(items)
        .active(ids.get(st.active).map(String::as_str))
        .viewport_width(cx.layout_size(&key).map(|z| z[0]))
        // C7's widths (`tabs.tab.*`), so a sheet's name shows as a browser tab's does.
        .overflow(TabOverflow::ShrinkThenScroll {
            min_width: slot::min_width(look),
            max_width: slot::max_width(look),
            pinned_width: slot::min_width(look),
            fade_width: slot::fade_width(look),
        })
        .label(st.labels.sheets.clone())
        .on_select(move |s: &mut S, id: &str, tx: &mut Tx| {
            let Some(i) = id.strip_prefix(&prefix).and_then(|n| n.parse().ok()) else {
                return;
            };
            if get_mut(s).show(i, tx)
                && let Some(f) = &intent
            {
                f(s, SheetIntent::Load(i), tx);
            }
        })
        .build(&look.kit);
    col()
        .w_full()
        .px(look.space.xs)
        .py(look.space.xs)
        .border_sides([1.0, 0.0, 0.0, 0.0], look.kit.c.border_muted)
        .child(strip)
}
