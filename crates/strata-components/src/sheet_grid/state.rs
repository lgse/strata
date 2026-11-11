//! [`SheetGridState`]: the sheets, and for each one its sort, widths, cursor, range and
//! scroll position, kept by the host and reached by accessor.

use std::cell::{Cell, OnceCell, RefCell};
use std::sync::Arc;
use std::time::Duration;

use rune::{Live, ScrollPos, Tx};
use rune_kit::SortDir;
use strata_kit::look::Look;

use super::copy;
use super::model::Sheet;
use super::range::{CellPos, CellRange, Columns, Selection};
use super::sort::{self, ColumnKeys};
use crate::collection::marquee::{EdgeScroll, edge_speed, glide_lead};
use crate::preview_drawer::{DocumentView, HostedPreview};
use crate::scroll::{Bar, ScrollFeed};

/// The glide a range drag's autoscroll rides, as C2's drag scroll does.
const GLIDE: rune::Spring = rune::tok::motion::GLIDE;
/// A frame's length when the clock hasn't moved, and the most one step may cover.
const FRAME: Duration = Duration::from_micros(16_667);
const MAX_FRAME: Duration = Duration::from_millis(50);

/// What the grid says, for Strata to translate. `{}` stands for the value each one names.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetLabels {
    /// The sheet tabs' list.
    pub sheets: String,
    /// The grid's name when its sheet has none (a CSV file).
    pub table: String,
    /// The corner over the row numbers.
    pub row: String,
    /// A column without a title, and a titled column's description: `{}` is its letters.
    pub column: String,
    /// A column divider: `{}` is the column's name.
    pub resize: String,
    /// A later sort key, in its column's description: `{}` is its place, 2 for the second.
    pub sort_key: String,
    pub ascending: String,
    pub descending: String,
    /// Strata's notice for a table its reader cut short.
    pub limited: String,
    /// Strata's description of a cell shown shortened.
    pub shortened: String,
    /// While a sheet's data is on its way (also the drawer's busy label).
    pub loading: String,
    /// Over the reason a sheet couldn't be read (Strata's title for a failed preview).
    pub failed: String,
    /// The button that asks for a sheet that couldn't be read once more.
    pub retry: String,
    /// A sheet with no rows.
    pub empty: String,
}

impl Default for SheetLabels {
    fn default() -> Self {
        SheetLabels {
            sheets: "Sheets".into(),
            table: "Table".into(),
            row: "Row".into(),
            column: "Column {}".into(),
            resize: "Resize {}".into(),
            sort_key: "Sort key {}".into(),
            ascending: "ascending".into(),
            descending: "descending".into(),
            limited: "Table preview reached its text, column, or value budget; open the file \
                      to see all data."
                .into(),
            shortened: "Cell shortened for responsive preview; copying the table keeps the \
                        complete text."
                .into(),
            loading: "Loading sheet…".into(),
            failed: "Preview unavailable".into(),
            retry: "Try again".into(),
            empty: "This sheet is empty".into(),
        }
    }
}

pub(crate) fn fill(template: &str, value: &str) -> String {
    template.replacen("{}", value, 1)
}

/// What the grid asks of the host.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SheetIntent {
    /// Sheet `i` was chosen and has no data yet: read it (Strata's sandbox) and hand it over
    /// with [`SheetGridState::set_sheet`], or say why not with
    /// [`SheetGridState::set_sheet_failed`]. Asked once until either; after a failure, again
    /// only when the person presses Try again.
    Load(usize),
}

/// One sheet: its data once it has come, and what the person did to it.
pub(crate) struct SheetView {
    pub name: Arc<str>,
    pub data: Option<Arc<Sheet>>,
    pub loading: bool,
    /// Why the sheet couldn't be read, until it is asked for again.
    pub failed: Option<String>,
    keys: Vec<OnceCell<ColumnKeys>>,
    pub spec: Vec<(usize, SortDir)>,
    /// Body rows (source indices after the title row) in display order.
    pub order: Vec<u32>,
    /// Each display row's element key, by position: the same keys in any order, so a re-sort
    /// changes none (Rune re-reads and copies every key of a list whose keys change, #203)
    /// and `virtual_rows_version` holds for the sheet. Built once.
    pub row_keys: Vec<String>,
    /// Every column's width once one has been resized by hand; until then the columns fit the
    /// viewport.
    pub manual: Option<Vec<f32>>,
    /// What each column wants, measured once per font size.
    pub wanted: RefCell<Option<(u32, Vec<f32>)>>,
    pub sel: Option<Selection>,
    /// The scroll position, kept while another sheet shows.
    pub offset: [f32; 2],
}

impl SheetView {
    fn new(name: Arc<str>) -> SheetView {
        SheetView {
            name,
            data: None,
            loading: false,
            failed: None,
            keys: Vec::new(),
            spec: Vec::new(),
            order: Vec::new(),
            row_keys: Vec::new(),
            manual: None,
            wanted: RefCell::new(None),
            sel: None,
            offset: [0.0, 0.0],
        }
    }

    fn fill(&mut self, key: &str, sheet: Sheet) {
        let cols = sheet.columns();
        let n = sheet.body().len();
        self.name = sheet.name.clone();
        self.keys = (0..cols).map(|_| OnceCell::new()).collect();
        self.row_keys = (0..n).map(|i| format!("{key}-r{i}")).collect();
        self.order = (0..n as u32).collect();
        self.data = Some(Arc::new(sheet));
        self.loading = false;
        self.failed = None;
        self.manual = None;
        *self.wanted.borrow_mut() = None;
        self.sel = None;
        self.spec.clear();
        if cols > 0 {
            self.sort(vec![(0, SortDir::Ascending)]);
        }
    }

    pub fn rows(&self) -> usize {
        self.order.len()
    }

    pub fn columns(&self) -> usize {
        self.keys.len()
    }

    fn keys(&self, column: usize) -> &ColumnKeys {
        let data = self.data.as_ref().expect("a sheet with keys has data");
        self.keys[column].get_or_init(|| ColumnKeys::new(data.body(), column))
    }

    /// Sorts by `spec`, reversing groups when only the primary's direction changed.
    fn sort(&mut self, spec: Vec<(usize, SortDir)>) {
        if spec == self.spec {
            return;
        }
        let flip = !self.spec.is_empty()
            && spec.len() == self.spec.len()
            && spec[0].0 == self.spec[0].0
            && spec[1..] == self.spec[1..];
        self.order = if flip {
            sort::flipped(&self.order, self.keys(spec[0].0))
        } else {
            let keys: Vec<(&ColumnKeys, SortDir)> =
                spec.iter().map(|&(c, d)| (self.keys(c), d)).collect();
            sort::order(self.order.len(), &keys)
        };
        self.spec = spec;
    }

    pub fn range(&self) -> Option<CellRange> {
        self.sel.and_then(|s| s.range())
    }
}

/// A range drag in progress: where the pointer is in the viewport, for the autoscroll.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DragState {
    pub pointer: [f32; 2],
    pub ticked: Option<f64>,
}

/// What the next tick does about a glide still under way when its drag ends or leaves the
/// edges: stop it dead where it is (Strata's stop, and C2's and C24's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Halt {
    #[default]
    None,
    Pending,
}

/// The sheet grid's state. The host makes one per preview, hands sheets over as they arrive
/// and calls [`Self::tick`] from its poll while the grid shows.
pub struct SheetGridState {
    pub(crate) key: String,
    pub(crate) sheets: Vec<SheetView>,
    pub(crate) active: usize,
    pub(crate) labels: SheetLabels,
    switch: Option<DocumentView>,
    pub(crate) feed: ScrollFeed,
    pub(crate) vbar: Bar,
    pub(crate) hbar: Bar,
    pub(crate) drag: Option<DragState>,
    pub(crate) halt: Halt,
    /// The last build's columns and row height, for the handlers' hit tests.
    pub(crate) laid: RefCell<Columns>,
    /// The last build's fitted widths, what a divider's double click puts a column back to.
    pub(crate) fitted: RefCell<Vec<f32>>,
    pub(crate) row_h: Cell<f32>,
    /// The columns built (column windowing, for wide sheets): `None` until the rows are first
    /// laid out, or after another sheet shows.
    pub(crate) window: Option<std::ops::Range<usize>>,
    /// The rows' viewport, `[x, y, w, h]` in the window, from its last layout.
    pub(crate) viewport: Cell<[f32; 4]>,
    /// The header titles' sideways offset: the rows' scroll, negated.
    pub(crate) strip_x: Live<[f32; 2]>,
    /// [`Self::set_active`] switched sheets with no `Tx` at hand: the next tick scrolls the
    /// active tab into the strip.
    reveal_tab: bool,
}

impl SheetGridState {
    /// An empty grid keyed `key`: its rows are `{key}-r{n}`, its column titles `{key}-h{j}`.
    pub fn new(key: impl Into<String>) -> SheetGridState {
        SheetGridState {
            key: key.into(),
            sheets: Vec::new(),
            active: 0,
            labels: SheetLabels::default(),
            switch: None,
            feed: ScrollFeed::new(),
            vbar: Bar::new(),
            hbar: Bar::new(),
            drag: None,
            halt: Halt::None,
            laid: RefCell::new(Columns::default()),
            fitted: RefCell::new(Vec::new()),
            row_h: Cell::new(28.0),
            window: None,
            viewport: Cell::new([0.0; 4]),
            strip_x: Live::new([0.0, 0.0]),
            reveal_tab: false,
        }
    }

    /// One sheet with no tabs: a CSV or TSV file.
    pub fn set_single(&mut self, sheet: Sheet) {
        let mut v = SheetView::new(sheet.name.clone());
        v.fill(&self.key, sheet);
        self.sheets = vec![v];
        self.active = 0;
        self.drag = None;
        self.window = None;
    }

    /// A workbook's sheets by name, none loaded yet; the first shows. The host loads the first
    /// itself, and the others when [`SheetIntent::Load`] asks.
    pub fn set_workbook(&mut self, names: impl IntoIterator<Item = Arc<str>>) {
        self.sheets = names.into_iter().map(SheetView::new).collect();
        self.active = 0;
        self.drag = None;
        if let Some(first) = self.sheets.first_mut() {
            first.loading = true;
        }
    }

    /// Sheet `i`'s data has come: it shows sorted by its first column, ascending.
    pub fn set_sheet(&mut self, i: usize, sheet: Sheet) {
        let key = self.key.clone();
        if let Some(v) = self.sheets.get_mut(i) {
            v.fill(&key, sheet);
            if i == self.active {
                self.drag = None;
                self.window = None;
            }
        }
    }

    /// Sheet `i` couldn't be read: the grid shows `reason` under Strata's "Preview
    /// unavailable", with a Try again button that asks [`SheetIntent::Load`] once more. The
    /// drawer stops showing it busy. Ignored for a sheet whose data has come.
    pub fn set_sheet_failed(&mut self, i: usize, reason: impl Into<String>) {
        if let Some(v) = self.sheets.get_mut(i).filter(|v| v.data.is_none()) {
            v.loading = false;
            v.failed = Some(reason.into());
        }
    }

    /// Why sheet `i` couldn't be read, while it shows as failed.
    pub fn failure(&self, i: usize) -> Option<&str> {
        self.sheets.get(i)?.failed.as_deref()
    }

    pub fn set_labels(&mut self, labels: SheetLabels) {
        self.labels = labels;
    }

    pub fn labels(&self) -> &SheetLabels {
        &self.labels
    }

    /// `Some(current)` offers the drawer's Rendered and Source switch (CSV and TSV have a
    /// source; workbooks don't).
    pub fn set_switch(&mut self, form: Option<DocumentView>) {
        self.switch = form;
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    /// Sheet `i`'s data, once it has come.
    pub fn sheet(&self, i: usize) -> Option<&Sheet> {
        self.sheets.get(i)?.data.as_deref()
    }

    pub(crate) fn view(&self) -> Option<&SheetView> {
        self.sheets.get(self.active)
    }

    pub(crate) fn view_mut(&mut self) -> Option<&mut SheetView> {
        self.sheets.get_mut(self.active)
    }

    /// Shows sheet `i` as it was left, without scrolling it (for a host restoring a preview);
    /// a tab click goes through the grid, which also keeps the scroll position. The sheet
    /// left drops its range, and the next [`Self::tick`] scrolls sheet `i`'s tab into view.
    pub fn set_active(&mut self, i: usize) {
        if i < self.sheets.len() {
            if i != self.active {
                self.clear_range();
            }
            self.active = i;
            self.drag = None;
            self.window = None;
            self.reveal_tab = true;
        }
    }

    /// Sorts the active sheet by `spec`, most recent first, as title clicks would leave it.
    pub fn set_sort(&mut self, spec: Vec<(usize, SortDir)>) {
        if let Some(v) = self.view_mut()
            && v.data.is_some()
            && spec.iter().all(|&(c, _)| c < v.columns())
        {
            v.sort(spec);
            v.sel = None;
        }
    }

    /// Selects from `anchor` to `cursor` (a range when they differ), clamped to the sheet.
    pub fn select(&mut self, anchor: CellPos, cursor: CellPos) {
        if let Some(v) = self.view_mut() {
            let (rows, cols) = (v.rows(), v.columns());
            if rows == 0 || cols == 0 {
                return;
            }
            let clamp = |c: CellPos| CellPos::new(c.row.min(rows - 1), c.col.min(cols - 1));
            v.sel = Some(Selection {
                anchor: clamp(anchor),
                cursor: clamp(cursor),
            });
        }
    }

    /// Whether sheet `i` was asked for and hasn't come.
    pub fn is_loading(&self, i: usize) -> bool {
        self.sheets.get(i).is_some_and(|v| v.loading)
    }

    /// The active sheet's sort, most recent first.
    pub fn sort(&self) -> &[(usize, SortDir)] {
        self.view().map_or(&[], |v| &v.spec)
    }

    /// The active sheet's body rows (source indices after the title row) in display order.
    pub fn order(&self) -> &[u32] {
        self.view().map_or(&[], |v| &v.order)
    }

    pub fn cursor(&self) -> Option<CellPos> {
        self.view()?.sel.map(|s| s.cursor)
    }

    pub fn range(&self) -> Option<CellRange> {
        self.view()?.range()
    }

    /// The range as Ctrl+C copies it, if there is one.
    pub fn copy_range(&self) -> Option<String> {
        let v = self.view()?;
        Some(copy::range_text(v.data.as_ref()?, &v.order, v.range()?))
    }

    /// The whole sheet as Ctrl+C copies it without a range.
    pub fn copy_all(&self) -> Option<String> {
        let v = self.view()?;
        Some(copy::all_text(v.data.as_ref()?, &v.order))
    }

    /// What Ctrl+C copies now: the range, else the whole sheet.
    pub fn copy_text(&self) -> Option<String> {
        self.copy_range().or_else(|| self.copy_all())
    }

    /// Column `j`'s width as last drawn.
    pub fn column_width(&self, j: usize) -> Option<f32> {
        let laid = self.laid.borrow();
        (j < laid.count()).then(|| {
            let (a, b) = laid.span(j);
            b - a
        })
    }

    /// Sets column `j`'s width as a drag does. The first resize keeps every other column at
    /// its drawn width from then on (Strata's rule).
    pub fn set_column_width(&mut self, j: usize, w: f32) {
        let drawn: Vec<f32> = {
            let laid = self.laid.borrow();
            (0..laid.count())
                .map(|k| laid.span(k))
                .map(|(a, b)| b - a)
                .collect()
        };
        let Some(v) = self.view_mut() else { return };
        let n = v.columns();
        let widths = v.manual.get_or_insert_with(|| {
            let mut out = drawn;
            out.resize(n, super::widths::MIN_WIDTH * 2.0);
            out
        });
        if let Some(slot) = widths.get_mut(j) {
            *slot = w.clamp(super::widths::MIN_WIDTH, super::widths::MAX_WIDTH);
        }
    }

    /// Puts column `j` back to its fitted width (a divider's double click, D7).
    pub fn fit_column(&mut self, j: usize) {
        let w = self.fitted.borrow().get(j).copied();
        if let (Some(w), Some(v)) = (w, self.view_mut())
            && let Some(slot) = v.manual.as_mut().and_then(|m| m.get_mut(j))
        {
            *slot = w;
        }
    }

    /// The range drag's autoscroll, one frame at `now` (seconds): near an edge of the rows the
    /// view glides that way (steps, with animations off) and the range follows the pointer.
    /// True when the range changed. Call it from the host's poll while the grid shows.
    pub fn tick(&mut self, now: f64, look: &Look, tx: &mut Tx) -> bool {
        if std::mem::take(&mut self.reveal_tab) {
            self.reveal_active_tab(tx);
        }
        if self.halt == Halt::Pending {
            self.halt = Halt::None;
            let p = self.feed.get();
            tx.scroll_to_instant(self.scroll_key(), p.offset[0], p.offset[1]);
        }
        let Some(mut drag) = self.drag else {
            return false;
        };
        let dt = drag
            .ticked
            .map(|t| now - t)
            .filter(|dt| *dt > 0.0)
            .map_or(FRAME, |dt| Duration::from_secs_f64(dt).min(MAX_FRAME));
        let p = self.feed.get();
        let gutter = self.laid.borrow().gutter;
        let e = EdgeScroll::STRATA;
        let towards = |pos: f32, size: f32, axis: usize| {
            let v = edge_speed(pos, size, e);
            let open = if v < 0.0 {
                p.offset[axis] > 0.5
            } else {
                p.offset[axis] < p.max[axis] - 0.5
            };
            if open { v } else { 0.0 }
        };
        let vx = towards(drag.pointer[0] - gutter, p.viewport[0] - gutter, 0);
        let vy = towards(drag.pointer[1], p.viewport[1], 1);
        if vx == 0.0 && vy == 0.0 {
            if drag.ticked.take().is_some() {
                tx.scroll_to_instant(self.scroll_key(), p.offset[0], p.offset[1]);
            }
            self.drag = Some(drag);
            return false;
        }
        drag.ticked = Some(now);
        self.drag = Some(drag);
        let scroll = self.scroll_key();
        let aim = |v: f32, o: f32| {
            if v == 0.0 {
                o
            } else if look.animations {
                o + glide_lead(v, GLIDE, dt)
            } else {
                o + v * dt.as_secs_f32()
            }
        };
        let (x, y) = (aim(vx, p.offset[0]), aim(vy, p.offset[1]));
        if look.animations {
            tx.scroll_to_with(scroll, x, y, GLIDE);
        } else {
            tx.scroll_to_instant(scroll, x, y);
        }
        tx.wake_after(Duration::ZERO);
        // The range follows the content now under the pointer.
        self.drag_to_viewport(drag.pointer, p)
    }

    /// Ends a range drag and its autoscroll. Call it when the grid stops showing (the drawer
    /// closed, another preview): Rune doesn't end a drag whose element goes (probe P4).
    pub fn stop(&mut self) {
        self.end_drag();
    }

    /// Ends the drag; a glide it started stops dead on the next tick.
    fn end_drag(&mut self) {
        if self.drag.take().is_some_and(|d| d.ticked.is_some()) {
            self.halt = Halt::Pending;
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    // --- Keys and paths ---------------------------------------------------------------------

    /// The active sheet's scroll viewport.
    pub fn scroll_key(&self) -> String {
        format!("{}-s{}-scroll", self.key, self.active)
    }

    /// Display row `i`'s element key.
    pub(crate) fn row_key(&self, i: usize) -> &str {
        self.view()
            .and_then(|v| v.row_keys.get(i))
            .map_or("", String::as_str)
    }

    pub(crate) fn list_key(&self) -> String {
        format!("{}-s{}-rows", self.key, self.active)
    }

    // --- What the handlers do ----------------------------------------------------------------

    /// The cell at a point of the rows' content, if it is on one. The row numbers stay at the
    /// viewport's left edge, over whatever has scrolled under them, so a point within the
    /// gutter's width of that edge is on them, at any sideways offset.
    pub(crate) fn cell_at(&self, x: f32, y: f32) -> Option<CellPos> {
        let v = self.view()?;
        let laid = self.laid.borrow();
        let h = self.row_h.get().max(1.0);
        let in_view = x - self.feed.get().offset[0];
        if v.rows() == 0 || y < 0.0 || x >= laid.width() || in_view < laid.gutter {
            return None;
        }
        let row = (y / h) as usize;
        (row < v.rows()).then(|| CellPos::new(row, laid.at(x)))
    }

    /// The cell nearest a point of the viewport, clamped to the sheet: where a drag's range
    /// reaches with the pointer over the header, the gutter or past the last row.
    fn clamped_at(&self, pointer: [f32; 2], p: ScrollPos) -> Option<CellPos> {
        let v = self.view()?;
        let laid = self.laid.borrow();
        if v.rows() == 0 || laid.count() == 0 {
            return None;
        }
        let x = (pointer[0].max(laid.gutter) + p.offset[0]).max(laid.gutter);
        let y = (pointer[1] + p.offset[1]).max(0.0);
        let row = ((y / self.row_h.get().max(1.0)) as usize).min(v.rows() - 1);
        Some(CellPos::new(row, laid.at(x)))
    }

    /// A press at content (`x`, `y`): a plain press puts the cursor there and clears the range;
    /// with Shift it spans the range from the anchor. Off every cell it clears the range.
    pub(crate) fn press(&mut self, x: f32, y: f32, shift: bool) {
        let at = self.cell_at(x, y);
        let p = self.feed.get();
        let pointer = [x - p.offset[0], y - p.offset[1]];
        let Some(v) = self.view_mut() else { return };
        match (at, v.sel) {
            (Some(c), Some(s)) if shift => {
                v.sel = Some(Selection {
                    anchor: s.anchor,
                    cursor: c,
                })
            }
            (Some(c), _) => v.sel = Some(Selection::at(c)),
            (None, Some(s)) => v.sel = Some(Selection::at(s.cursor)),
            (None, None) => {}
        }
        if at.is_some() {
            self.drag = Some(DragState {
                pointer,
                ticked: None,
            });
        }
    }

    /// The drag moved to content (`x`, `y`). True when the range changed.
    pub(crate) fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let p = self.feed.get();
        let pointer = [x - p.offset[0], y - p.offset[1]];
        let Some(drag) = &mut self.drag else {
            return false;
        };
        drag.pointer = pointer;
        self.drag_to_viewport(pointer, p)
    }

    fn drag_to_viewport(&mut self, pointer: [f32; 2], p: ScrollPos) -> bool {
        let Some(c) = self.clamped_at(pointer, p) else {
            return false;
        };
        let Some(v) = self.view_mut() else {
            return false;
        };
        let Some(s) = &mut v.sel else { return false };
        let changed = s.cursor != c;
        s.cursor = c;
        changed
    }

    pub(crate) fn release(&mut self) {
        self.end_drag();
    }

    /// Clears the range, keeping the cursor. True when there was one.
    pub(crate) fn clear_range(&mut self) -> bool {
        let Some(v) = self.view_mut() else {
            return false;
        };
        match v.sel {
            Some(s) if s.range().is_some() => {
                v.sel = Some(Selection::at(s.cursor));
                true
            }
            _ => false,
        }
    }

    /// A click on column `j`'s title: the sort that makes (see [`sort::clicked`]), the range
    /// cleared and the rows back at the top, as Strata does.
    pub(crate) fn click_title(&mut self, j: usize, tx: &mut Tx) {
        let scroll = self.scroll_key();
        let x = self.feed.get().offset[0];
        let Some(v) = self.view_mut() else { return };
        if v.data.is_none() || j >= v.columns() {
            return;
        }
        let spec = sort::clicked(&v.spec, j);
        v.sort(spec);
        v.sel = v.sel.map(|s| Selection::at(CellPos::new(0, s.cursor.col)));
        tx.scroll_to_instant(scroll, x, 0.0);
    }

    /// Shows sheet `i`, keeping where the last one was scrolled and its cursor, but not its
    /// range, and scrolls its tab into view. True when the host should load it.
    pub(crate) fn show(&mut self, i: usize, tx: &mut Tx) -> bool {
        if i >= self.sheets.len() || i == self.active {
            return false;
        }
        let offset = self.feed.get().offset;
        self.clear_range();
        if let Some(v) = self.view_mut() {
            v.offset = offset;
        }
        self.drag = None;
        self.window = None;
        self.active = i;
        self.feed.set(ScrollPos::default());
        let back = self.sheets[i].offset;
        self.strip_x.set([-back[0], 0.0]);
        if back != [0.0, 0.0] {
            tx.scroll_to_instant(self.scroll_key(), back[0], back[1]);
        }
        self.reveal_active_tab(tx);
        let v = &mut self.sheets[i];
        let load = v.data.is_none() && !v.loading && v.failed.is_none();
        v.loading |= load;
        load
    }

    /// Scrolls the active sheet's tab into the strip: `TabStrip` leaves that to the app, as
    /// C7's tabs do. Rune's reveal stops at the strip's edge, under its fade (C7's U-C7-7).
    fn reveal_active_tab(&self, tx: &mut Tx) {
        if self.sheets.len() >= 2 {
            let key = &self.key;
            tx.scroll_into_view_in(
                format!("{key}-tabs-scroll"),
                format!("t-{key}-sheet{}", self.active),
            );
        }
    }

    /// Try again on the active sheet's failure: it shows loading again. The sheet the host
    /// should load, if it had failed.
    pub(crate) fn retry(&mut self) -> Option<usize> {
        let i = self.active;
        let v = self.view_mut()?;
        v.failed.take()?;
        v.loading = true;
        Some(i)
    }

    /// The rows moved to `list_x` in the window (a scroll, sideways or down). True when the
    /// columns that show have left the built ones, which are then rebuilt around them: a
    /// commit about once a viewport width of sideways travel, none for a scroll down.
    pub(crate) fn follow_columns(&mut self, list_x: f32) -> bool {
        let vp = self.viewport.get();
        self.cover_columns((vp[0] - list_x).max(0.0))
    }

    /// The viewport laid out at `r` (it moved or resized). True when the built columns no
    /// longer cover what shows.
    pub(crate) fn viewport_laid(&mut self, r: [f32; 4]) -> bool {
        self.viewport.set(r);
        self.cover_columns(self.feed.get().offset[0])
    }

    fn cover_columns(&mut self, offset: f32) -> bool {
        let vp = self.viewport.get();
        if vp[2] <= 0.0 {
            // Not laid out yet: the build windows from the layout it reads.
            return false;
        }
        let laid = self.laid.borrow();
        let shows = laid.showing(offset, vp[2]);
        match &self.window {
            Some(w) if w.start <= shows.start && shows.end <= w.end => false,
            _ => {
                let w = laid.window(offset, vp[2]);
                drop(laid);
                self.window = Some(w);
                true
            }
        }
    }

    /// The columns to build now.
    pub(crate) fn built_columns(&self, cols: &Columns, width: f32) -> std::ops::Range<usize> {
        match &self.window {
            Some(w) if w.end <= cols.count() => w.clone(),
            _ => cols.window(self.feed.get().offset[0], width),
        }
    }

    /// Moves the cursor to `c` (with `extend`, the range grows from the anchor) and scrolls it
    /// into view.
    pub(crate) fn move_cursor(&mut self, c: CellPos, extend: bool, tx: &mut Tx) {
        // The cursor's cell is built, so the grid's active descendant is there to name.
        let vp = self.viewport.get();
        let around = {
            let laid = self.laid.borrow();
            (c.col < laid.count() && !self.window.as_ref().is_some_and(|w| w.contains(&c.col)))
                .then(|| laid.window((laid.span(c.col).0 - vp[2] / 2.0).max(0.0), vp[2]))
        };
        if around.is_some() {
            self.window = around;
        }
        let Some(v) = self.view_mut() else { return };
        v.sel = Some(match v.sel {
            Some(s) if extend => Selection {
                anchor: s.anchor,
                cursor: c,
            },
            _ => Selection::at(c),
        });
        self.reveal(c, tx);
    }

    /// Scrolls the least that brings cell `c` wholly into view, past the sticky gutter.
    fn reveal(&self, c: CellPos, tx: &mut Tx) {
        let p = self.feed.get();
        let laid = self.laid.borrow();
        if c.col >= laid.count() {
            return;
        }
        let h = self.row_h.get();
        let into = |start: f32, end: f32, offset: f32, size: f32, inset: f32| {
            if start - inset < offset {
                start - inset
            } else if end > offset + size {
                (end - size).min(start - inset)
            } else {
                offset
            }
        };
        let (a, b) = laid.span(c.col);
        let x = into(a, b, p.offset[0], p.viewport[0], laid.gutter).max(0.0);
        let top = c.row as f32 * h;
        let y = into(top, top + h, p.offset[1], p.viewport[1], 0.0).max(0.0);
        if [x, y] != p.offset {
            tx.scroll_to(self.scroll_key(), x, y);
        }
    }
}

impl HostedPreview for SheetGridState {
    fn document_view(&self) -> Option<DocumentView> {
        self.switch
    }

    fn busy(&self) -> Option<&str> {
        self.view()
            .is_some_and(|v| v.loading)
            .then_some(self.labels.loading.as_str())
    }
}
