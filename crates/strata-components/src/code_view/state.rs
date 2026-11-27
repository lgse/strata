// SPDX-License-Identifier: MIT

use std::cell::{Cell, OnceCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rune::{TextSelection, Tx};

use super::copy;
use super::find::Found;
use super::gutter::{self, GutterDrag, RowHighlights};
use super::position::Position;
use super::rows::Rows;
use crate::code_view::highlight::{CodeDoc, Highlighter, Source};
use crate::document_view::Folded;
use crate::preview_drawer::{DocumentView, FindStatus, HostedPreview};
use crate::scroll::Bar;

static SERIAL: AtomicU64 = AtomicU64::new(1);

fn serial() -> u64 {
    SERIAL.fetch_add(1, Ordering::Relaxed)
}

/// A selection or scroll find asks for, applied by [`CodeState::poll`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    Select(String, Range<usize>),
    Clear,
    /// Centre the current match, by its row's key.
    Reveal(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpectedSelect {
    /// Single-row Rune selection `(row, start, end)` on `{key}-l{row}/t`.
    Row(usize, usize, usize),
    GutterSentinel,
}

#[derive(Debug)]
pub(crate) struct Content {
    pub key: String,
    pub rows: Rows,
    pub keys: Vec<Box<str>>,
    /// The first and last rows with text: where a whole-text selection ends.
    pub ends: Option<(usize, usize)>,
    /// Ctrl+A selected the whole text, and nothing has changed the selection since.
    pub all: Cell<bool>,
    /// Active keyboard or gutter-drag byte range `(lo, hi)` when `lo < hi`.
    pub keyboard_range: Cell<Option<(usize, usize)>>,
    pub expected_select: Cell<Option<ExpectedSelect>>,
}

impl Content {
    fn new(key: String, text: Arc<str>) -> Content {
        let rows = Rows::new(text);
        let first = (0..rows.len()).find(|&r| !rows.source(r).is_empty());
        let last = (0..rows.len()).rev().find(|&r| !rows.source(r).is_empty());
        let keys = (0..rows.len())
            .map(|r| format!("{key}-l{r}").into_boxed_str())
            .collect();
        Content {
            key,
            ends: first.zip(last),
            rows,
            keys,
            all: Cell::new(false),
            keyboard_range: Cell::new(None),
            expected_select: Cell::new(None),
        }
    }

    pub fn copy(&self, sel: &TextSelection) -> String {
        copy::copy(self, sel)
    }
}

/// How the text is coloured: the app's highlighter and what detection reads besides the text.
#[derive(Clone)]
struct Highlight {
    highlighter: Highlighter,
    file_name: Option<String>,
    syntax: Option<String>,
}

pub struct CodeState {
    content: Rc<Content>,
    revision: u64,
    wrap: bool,
    line_numbers: bool,
    notices: Vec<String>,
    document_view: Option<DocumentView>,
    highlight: Option<Highlight>,
    doc: Option<CodeDoc>,
    query: String,
    found: Found,
    current: Option<usize>,
    folded: OnceCell<Folded>,
    pending: Vec<Request>,
    /// Rune's visible rows, as of the last report.
    pub(crate) visible: Range<usize>,
    /// Read-only keyboard caret `(anchor, head)` byte offsets in `rows.text`.
    caret: Option<(usize, usize)>,
    /// Logical line anchored by the most recent gutter click (preserved for Shift+click).
    gutter_anchor: Option<usize>,
    /// Active gutter drag gesture, if the primary button is currently held on the gutter.
    gutter_drag: Cell<Option<GutterDrag>>,
    /// Live-only row fills for whole-line gutter selections.
    pub(crate) highlights: RowHighlights,
    pub(crate) position: Position,
    pub(crate) sideways: Bar,
    /// Whether the text overflows the viewport, as the view last drew the position bar.
    pub(crate) overflow: bool,
    /// The position the bar's accessible value last said, in steps of a two-hundredth.
    pub(crate) said: u32,
    last_scroll_offset: f32,
    /// Clock deadline after scrolling pauses when `said` should commit its updated value.
    scroll_settled_at: Option<f64>,
    reveal_again: Option<String>,
}

impl std::fmt::Debug for CodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeState")
            .field("key", &self.content.key)
            .field("rows", &self.content.rows.len())
            .field("wrap", &self.wrap)
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl Default for CodeState {
    fn default() -> Self {
        CodeState::new("")
    }
}

impl CodeState {
    /// A preview of `text`, plain until [`highlighter`](Self::highlighter) is set.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        CodeState {
            content: Rc::new(Content::new("code".into(), text.into())),
            revision: serial(),
            wrap: true,
            line_numbers: true,
            notices: Vec::new(),
            document_view: None,
            highlight: None,
            doc: None,
            query: String::new(),
            found: Found::default(),
            current: None,
            folded: OnceCell::new(),
            pending: Vec::new(),
            visible: 0..0,
            caret: None,
            gutter_anchor: None,
            gutter_drag: Cell::new(None),
            highlights: RowHighlights::default(),
            position: Position::default(),
            sideways: Bar::new(),
            overflow: false,
            said: u32::MAX,
            last_scroll_offset: 0.0,
            scroll_settled_at: None,
            reveal_again: None,
        }
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.content = Rc::new(Content::new(key.into(), self.content.rows.text.clone()));
        self
    }

    pub fn highlighter(
        mut self,
        highlighter: &Highlighter,
        file_name: Option<&str>,
        syntax: Option<&str>,
    ) -> Self {
        self.highlight = Some(Highlight {
            highlighter: highlighter.clone(),
            file_name: file_name.map(str::to_owned),
            syntax: syntax.map(str::to_owned),
        });
        self.open_doc();
        self
    }

    fn open_doc(&mut self) {
        self.doc = self.highlight.as_ref().map(|h| {
            let mut src = Source::new(self.content.rows.text.clone());
            src.file_name = h.file_name.clone();
            src.syntax = h.syntax.clone();
            h.highlighter.open(src)
        });
        if !self.visible.is_empty() {
            let lines = self.content.rows.lines_of(self.visible.clone());
            if let Some(doc) = &mut self.doc {
                doc.show(lines);
            }
        }
    }

    pub fn view_key(&self) -> &str {
        &self.content.key
    }

    pub fn text(&self) -> &Arc<str> {
        &self.content.rows.text
    }

    pub fn set_text(&mut self, text: impl Into<Arc<str>>) {
        self.content = Rc::new(Content::new(self.content.key.clone(), text.into()));
        self.revision = serial();
        self.caret = None;
        self.gutter_anchor = None;
        self.gutter_drag.set(None);
        self.highlights.reset();
        self.folded = OnceCell::new();
        self.found = self.run_find();
        self.current = None;
        self.pending.retain(|r| *r == Request::Clear);
        self.position.reset();
        self.said = u32::MAX;
        self.last_scroll_offset = 0.0;
        self.scroll_settled_at = None;
        self.reveal_again = None;
        self.open_doc();
    }

    /// New for every state and every [`set_text`](Self::set_text).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The wrap preference: on by default. Off, the text scrolls sideways as one.
    pub fn set_wrap(&mut self, wrap: bool) {
        self.wrap = wrap;
    }

    pub fn wraps(&self) -> bool {
        self.wrap
    }

    /// Line numbers in the gutter: on by default.
    pub fn set_line_numbers(&mut self, on: bool) {
        self.line_numbers = on;
        if !on {
            self.stop_gutter_drag();
        }
    }

    pub(crate) fn line_numbers(&self) -> bool {
        self.line_numbers
    }

    pub fn set_notices(&mut self, notices: Vec<String>) {
        self.notices = notices;
    }

    pub(crate) fn notices(&self) -> &[String] {
        &self.notices
    }

    pub fn set_document_view(&mut self, form: Option<DocumentView>) {
        self.document_view = form;
    }

    pub(crate) fn content(&self) -> Rc<Content> {
        self.content.clone()
    }

    pub(crate) fn rows(&self) -> &Rows {
        &self.content.rows
    }

    /// The viewport's latest [`rune::ScrollPos`].
    pub fn scroll_pos(&self) -> rune::ScrollPos {
        self.position.scroll_pos()
    }

    /// The viewport's vertical offset, as Rune last reported it.
    pub fn scroll_offset(&self) -> f32 {
        self.position.offset()
    }

    /// Whether the view has any text to select.
    pub fn has_text(&self) -> bool {
        !self.content.rows.text.is_empty()
    }

    /// The currently selected text belonging to this view, if any.
    pub fn selected_text(&self, sel: Option<&TextSelection>) -> Option<String> {
        if self.all_selected() && sel.is_some_and(|s| copy::reads_whole(&self.content, s)) {
            return self.has_text().then(|| self.copy_all());
        }
        if copy::matches_expected(&self.content, sel)
            && let Some(text) = self.copy_keyboard()
            && !text.is_empty()
        {
            return Some(text);
        }
        let sel = sel?;
        copy::span(&self.content, sel)
            .and_then(|(a, b)| (a < b).then(|| self.content.rows.text[a..b].to_owned()))
    }

    /// Whether `sel` currently selects the whole text.
    pub fn selects_whole(&self, sel: Option<&TextSelection>) -> bool {
        sel.is_some_and(|s| copy::reads_whole(&self.content, s))
    }

    /// Byte span `(lo, hi)` currently selected in this view, if any.
    pub fn selection_span(&self, sel: Option<&TextSelection>) -> Option<(usize, usize)> {
        if copy::matches_expected(&self.content, sel)
            && let Some(span) = self.keyboard_selection()
        {
            return Some(span);
        }
        sel.and_then(|s| copy::span(&self.content, s))
            .filter(|(a, b)| a < b)
    }

    pub fn restore_selection_range(&mut self, lo: usize, hi: usize, tx: &mut Tx) {
        let text = &self.content.rows.text;
        if lo < hi && hi <= text.len() && text.is_char_boundary(lo) && text.is_char_boundary(hi) {
            if self.line_numbers
                && let Some((lo_line, hi_line)) = self.content.rows.line_bounds_if_whole(lo, hi)
            {
                self.apply_gutter_lines(lo_line, hi_line, tx);
            } else {
                self.highlights.clear();
                self.apply_caret_inner(lo, hi, tx);
            }
        }
    }

    /// Selects the entire document text in Rune and arms model-backed whole-text copying.
    pub fn select_all_text(&mut self, tx: &mut Tx) {
        self.content.all.set(self.has_text());
        self.content.keyboard_range.set(None);
        self.content.expected_select.set(None);
        self.gutter_anchor = None;
        self.gutter_drag.set(None);
        self.highlights.clear();
        self.caret = Some((0, self.content.rows.text.len()));
        tx.select_all_text(self.list_key());
    }

    /// Scrolls the viewport to the start (`end == false`) or end (`end == true`) of the text.
    pub fn scroll_to_edge(&self, end: bool, tx: &mut Tx) {
        self.position
            .go_step(if end { f64::INFINITY } else { 0.0 }, tx);
    }

    /// Whether line `line`'s colours have landed (a line with no grammar never colours).
    pub fn is_coloured(&self, line: usize) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|d| d.line_spans(line).is_some())
    }

    pub(crate) fn doc(&self) -> Option<&CodeDoc> {
        self.doc.as_ref()
    }

    pub(crate) fn row_key(&self, r: usize) -> String {
        format!("{}-l{r}", self.content.key)
    }

    /// Row `r`'s key, borrowed.
    pub(crate) fn row_key_ref(&self, r: usize) -> &str {
        &self.content.keys[r]
    }

    pub(crate) fn list_key(&self) -> String {
        format!("{}-rows", self.content.key)
    }

    pub fn scroll_key(&self) -> String {
        format!("{}-scroll", self.content.key)
    }

    pub(crate) fn gutter_sentinel_key(&self) -> String {
        format!("{}-gutter-sel", self.content.key)
    }

    /// Rune's visible rows: asks for the colours of the lines they show.
    pub(crate) fn set_visible(&mut self, rows: Range<usize>) {
        self.visible = rows.clone();
        self.position.set_visible(rows.clone());
        let n = rows.len().max(1);
        let keep = rows.start.saturating_sub(3 * n)..rows.end + 3 * n;
        self.highlights.retain(keep);
        let lines = self.content.rows.lines_of(rows);
        if let Some(doc) = &mut self.doc {
            doc.show(lines);
        }
    }

    pub fn poll(&mut self, tx: &mut Tx) -> bool {
        if self.gutter_drag.get().is_none()
            && self.highlights.lines().is_some()
            && !copy::matches_expected(&self.content, tx.text_selection())
        {
            self.clear_stale_keyboard_selection(tx.text_selection());
        }
        gutter::poll_gutter_drag(self, tx);
        let mut changed = self.doc.as_mut().is_some_and(CodeDoc::poll);
        if let Some(row) = self.reveal_again.take() {
            tx.scroll_center_in(self.scroll_key(), format!("{row}/current"));
        }
        for r in std::mem::take(&mut self.pending) {
            changed = true;
            match r {
                Request::Select(key, range) => tx.select_text(key, range),
                Request::Clear => tx.clear_text_selection(),
                Request::Reveal(row) => {
                    tx.scroll_center_in(self.scroll_key(), format!("{row}/current"));
                    self.reveal_again = Some(row);
                    tx.wake_after(std::time::Duration::ZERO);
                }
            }
        }
        let overflow = self.position.overflows();
        if overflow != self.overflow {
            self.overflow = overflow;
            changed = true;
        }
        changed |= self.poll_said(changed, tx);
        changed
    }

    fn poll_said(&mut self, already_changed: bool, tx: &mut Tx) -> bool {
        /// Quiet interval after the last scroll step before rebuilding to update `a11y_value`.
        const SAID_SETTLE_SECS: f64 = 0.15;

        let said = (self.position.progress() * 200.0).round() as u32;
        let offset = self.position.offset();
        let moved = (offset - self.last_scroll_offset).abs() > 0.5;
        self.last_scroll_offset = offset;

        if self.said == u32::MAX || already_changed {
            self.said = said;
            self.scroll_settled_at = None;
            return false;
        }
        if self.position.moved_by_step() {
            self.said = said;
            self.scroll_settled_at = None;
            return true;
        }
        if said == self.said {
            self.scroll_settled_at = None;
            return false;
        }
        let now = tx.clock();
        let active = moved || self.position.dragging() || self.gutter_auto_scrolling();
        if !active && self.scroll_settled_at.is_some_and(|d| now >= d - 1e-3) {
            self.said = said;
            self.scroll_settled_at = None;
            return true;
        }
        let due = self
            .scroll_settled_at
            .filter(|_| !active)
            .unwrap_or(now + SAID_SETTLE_SECS);
        self.scroll_settled_at = Some(due);
        tx.wake_at(due);
        false
    }

    pub fn settle(&mut self, lines: Range<usize>, timeout: std::time::Duration) -> bool {
        match &mut self.doc {
            Some(doc) => {
                doc.show(lines);
                doc.settle(timeout)
            }
            None => true,
        }
    }

    pub(crate) fn all_selected(&self) -> bool {
        self.content.all.get()
    }

    pub(crate) fn clear_all_selected(&mut self) {
        self.content.all.set(false);
    }

    pub(crate) fn reads_whole(&self, sel: &TextSelection) -> bool {
        copy::reads_whole(&self.content, sel)
    }

    pub(crate) fn on_pointer_press(&mut self) {
        self.content.all.set(false);
        self.content.keyboard_range.set(None);
        self.content.expected_select.set(None);
        self.caret = None;
        self.gutter_anchor = None;
        self.gutter_drag.set(None);
        self.highlights.clear();
    }

    pub(crate) fn gutter_anchor(&self) -> Option<usize> {
        self.gutter_anchor
    }

    pub(crate) fn set_gutter_anchor(&mut self, line: Option<usize>) {
        self.gutter_anchor = line;
    }

    pub(crate) fn gutter_drag(&self) -> Option<GutterDrag> {
        self.gutter_drag.get()
    }

    pub(crate) fn set_gutter_drag(&self, drag: Option<GutterDrag>) {
        self.gutter_drag.set(drag);
    }

    /// Stops any active gutter drag and its edge auto-scroll timer while keeping the selection.
    pub(crate) fn stop_gutter_drag(&self) {
        self.gutter_drag.set(None);
    }

    fn gutter_auto_scrolling(&self) -> bool {
        self.gutter_drag
            .get()
            .is_some_and(|d| d.last_tick.is_some())
    }

    /// Inclusive logical-line range currently highlighted by a gutter selection, if any.
    #[cfg(test)]
    pub(crate) fn gutter_highlighted_lines(&self) -> Option<(usize, usize)> {
        self.highlights.lines()
    }

    fn clear_stale_keyboard_selection(&mut self, sel: Option<&TextSelection>) {
        self.content.keyboard_range.set(None);
        self.content.expected_select.set(None);
        self.gutter_anchor = None;
        self.highlights.clear();
        self.caret = sel.and_then(|s| copy::span(&self.content, s));
    }

    pub(crate) fn sync_caret_from_selection(&mut self, sel: Option<&TextSelection>) {
        if self.content.all.get() {
            if sel.is_some_and(|s| copy::reads_whole(&self.content, s)) {
                return;
            }
            self.content.all.set(false);
            self.gutter_anchor = None;
            self.highlights.clear();
            self.caret = sel.and_then(|s| copy::span(&self.content, s));
            return;
        }
        if self.content.keyboard_range.get().is_some() {
            if copy::matches_expected(&self.content, sel) {
                return;
            }
            self.clear_stale_keyboard_selection(sel);
            return;
        }
        if let Some(span) = sel.and_then(|s| copy::span(&self.content, s)) {
            self.caret = Some(span);
        }
    }

    pub(crate) fn caret(&self) -> Option<(usize, usize)> {
        self.caret
    }

    pub fn keyboard_selection(&self) -> Option<(usize, usize)> {
        self.content.keyboard_range.get()
    }

    pub fn keyboard_selection_for(&self, sel: Option<&TextSelection>) -> Option<(usize, usize)> {
        copy::matches_expected(&self.content, sel)
            .then(|| self.content.keyboard_range.get())
            .flatten()
    }

    pub(crate) fn caret_or_selection(&self, sel: Option<&TextSelection>) -> (usize, usize) {
        if let Some(pair) = self.caret {
            return pair;
        }
        sel.and_then(|s| copy::span(&self.content, s))
            .unwrap_or((0, 0))
    }

    pub(crate) fn apply_caret(&mut self, anchor: usize, head: usize, tx: &mut Tx) {
        self.gutter_anchor = None;
        self.highlights.clear();
        self.apply_caret_inner(anchor, head, tx);
    }

    pub(crate) fn apply_gutter_lines(
        &mut self,
        anchor_line: usize,
        target_line: usize,
        tx: &mut Tx,
    ) {
        let last = self.content.rows.lines().saturating_sub(1);
        let anchor_line = anchor_line.min(last);
        let target_line = target_line.min(last);
        let (lo, hi) = self.content.rows.line_range_bytes(anchor_line, target_line);
        let (anchor, head) = if target_line >= anchor_line {
            (lo, hi)
        } else {
            (hi, lo)
        };
        let range = (lo < hi).then_some((lo, hi));
        let lines = (lo < hi).then(|| (anchor_line.min(target_line), anchor_line.max(target_line)));
        let sentinel_synced = self.content.expected_select.get()
            == Some(ExpectedSelect::GutterSentinel)
            && copy::matches_expected(&self.content, tx.text_selection());
        if self.caret == Some((anchor, head))
            && self.content.keyboard_range.get() == range
            && self.highlights.lines() == lines
            && (range.is_none() || sentinel_synced)
        {
            return;
        }
        self.caret = Some((anchor, head));
        self.content.all.set(false);
        self.content.keyboard_range.set(range);
        if range.is_some() {
            self.content
                .expected_select
                .set(Some(ExpectedSelect::GutterSentinel));
            if !sentinel_synced {
                tx.select_text(self.gutter_sentinel_key(), 0..1);
            }
        } else {
            self.content.expected_select.set(None);
            tx.clear_text_selection();
        }
        self.highlights.set_lines(lines, &self.content.rows);
    }

    fn apply_caret_inner(&mut self, anchor: usize, head: usize, tx: &mut Tx) {
        let len = self.content.rows.text.len();
        let anchor = anchor.min(len);
        let head = head.min(len);
        self.caret = Some((anchor, head));
        self.content.all.set(false);
        let (lo, hi) = (anchor.min(head), anchor.max(head));
        if lo < hi {
            let prev_expected = self.content.expected_select.get();
            let already_synced = copy::matches_expected(&self.content, tx.text_selection());
            self.content.keyboard_range.set(Some((lo, hi)));
            if let Some((r, range)) = self.first_nonempty_row_slice(lo, hi) {
                let next = ExpectedSelect::Row(r, range.start, range.end);
                self.content.expected_select.set(Some(next));
                if !(already_synced && prev_expected == Some(next)) {
                    tx.select_text(format!("{}/t", self.row_key(r)), range);
                }
            } else {
                self.content.expected_select.set(None);
                if !(already_synced && prev_expected.is_none()) {
                    tx.clear_text_selection();
                }
            }
        } else {
            self.content.keyboard_range.set(None);
            self.content.expected_select.set(None);
            tx.clear_text_selection();
        }
    }

    fn first_nonempty_row_slice(&self, lo: usize, hi: usize) -> Option<(usize, Range<usize>)> {
        let start_row = self.content.rows.row_at(lo);
        let end_row = self.content.rows.row_at(hi);
        for r in start_row..=end_row {
            let row = self.content.rows.row(r);
            let a = lo.max(row.start);
            let b = hi.min(row.end);
            if a < b {
                return Some((r, (a - row.start)..(b - row.start)));
            }
        }
        None
    }

    pub(crate) fn copy_keyboard(&self) -> Option<String> {
        let (a, b) = self.content.keyboard_range.get()?;
        let text = &self.content.rows.text;
        (a < b && b <= text.len() && text.is_char_boundary(a) && text.is_char_boundary(b))
            .then(|| text[a..b].to_owned())
    }

    pub(crate) fn copy_keyboard_if_valid(&mut self, sel: Option<&TextSelection>) -> Option<String> {
        self.content.keyboard_range.get()?;
        if copy::matches_expected(&self.content, sel) {
            return self.copy_keyboard();
        }
        self.clear_stale_keyboard_selection(sel);
        None
    }

    /// What copying `sel` takes: the text between its ends, from the model.
    pub fn copy_selection(&self, sel: &TextSelection) -> String {
        self.content.copy(sel)
    }

    /// The whole text, as Ctrl+A then Ctrl+C copies it.
    pub fn copy_all(&self) -> String {
        self.content.rows.text.to_string()
    }

    /// The matches of the open find, as byte ranges of the text.
    pub fn matches(&self) -> &[Range<usize>] {
        &self.found.ranges
    }

    pub(crate) fn found(&self) -> &Found {
        &self.found
    }

    /// The selected match's index.
    pub fn current_match(&self) -> Option<usize> {
        self.current
    }

    fn run_find(&self) -> Found {
        if self.query.is_empty() {
            return Found::default();
        }
        let folded = self
            .folded
            .get_or_init(|| Folded::new(&self.content.rows.text));
        Found::new(folded, &self.query)
    }

    fn select_current(&mut self) {
        let Some(m) = self.current.map(|c| self.found.ranges[c].clone()) else {
            return;
        };
        let rows = &self.content.rows;
        let r = rows.row_at(m.start);
        let row = rows.row(r);
        let a = m.start - row.start;
        let b = m.end.min(row.end) - row.start;
        let key = self.row_key(r);
        self.pending.push(Request::Reveal(key.clone()));
        self.pending.push(Request::Select(format!("{key}/t"), a..b));
    }

    #[cfg(test)]
    pub(crate) fn pending(&self) -> &[Request] {
        &self.pending
    }
}

impl HostedPreview for CodeState {
    fn document_view(&self) -> Option<DocumentView> {
        self.document_view
    }

    fn wrap(&self) -> Option<bool> {
        Some(self.wrap)
    }

    fn searchable(&self) -> bool {
        !self.content.rows.text.is_empty()
    }

    fn find(&mut self, query: &str) {
        self.query = query.to_owned();
        self.found = self.run_find();
        self.current = (!self.found.ranges.is_empty()).then_some(0);
        // Each edit to the query clears the selection; the first match then takes it.
        self.pending.push(Request::Clear);
        self.on_pointer_press();
        self.select_current();
    }

    fn find_step(&mut self, forward: bool) {
        let n = self.found.ranges.len();
        if n == 0 {
            return;
        }
        self.current = Some(match self.current {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None => 0,
        });
        self.on_pointer_press();
        self.select_current();
    }

    fn find_status(&self) -> Option<FindStatus> {
        Some(FindStatus {
            current: self.current.map(|c| c + 1),
            total: self.found.ranges.len(),
        })
    }

    fn find_end(&mut self) {
        // The selected match stays selected for Ctrl+C.
        self.query.clear();
        self.found = Found::default();
        self.current = None;
    }
}
