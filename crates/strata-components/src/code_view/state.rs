//! [`CodeState`]: what the host keeps per source preview, and what C9's header and find bar
//! ask of it.

use std::cell::{Cell, OnceCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rune::{TextSelection, Tx};

use super::copy;
use super::find::Found;
use super::position::Position;
use super::rows::Rows;
use crate::code_view::highlight::{CodeDoc, Highlighter, Source};
use crate::document_view::Folded;
use crate::preview_drawer::{DocumentView, FindStatus, HostedPreview};
use crate::scroll::Bar;

/// Each text a state shows gets a serial, so a view of a new state or text asks Rune for its
/// visible rows again even where the row count and keys match the last one's.
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

/// What doesn't change between [`CodeState::set_text`]s. The view's copy handler holds it,
/// since Rune gives that handler no app state.
#[derive(Debug)]
pub(crate) struct Content {
    pub key: String,
    pub rows: Rows,
    /// Each row's key, `{key}-l{row}`, made once: Rune reads every key on a list's first
    /// commit, and borrowing them keeps that commit short.
    pub keys: Vec<Box<str>>,
    /// The first and last rows with text: where a whole-text selection ends.
    pub ends: Option<(usize, usize)>,
    /// Ctrl+A selected the whole text, and nothing has changed the selection since.
    pub all: Cell<bool>,
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

/// One source preview's state, kept by the host and given to
/// [`CodeView`](super::CodeView) by accessor.
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
    /// Ctrl+A asked Rune to select everything from a selection that didn't hold it, and the
    /// selection change that makes hasn't come yet.
    all_pending: bool,
    pub(crate) position: Position,
    /// The sideways bar while wrap is off (C24's).
    pub(crate) sideways: Bar,
    /// Whether the text overflows the viewport, as the view last drew the position bar.
    pub(crate) overflow: bool,
    /// The position the bar's accessible value last said, in steps of a two-hundredth.
    pub(crate) said: u32,
    /// The current match to centre again on the next poll: find opening its bar in the same
    /// frame moves the viewport's bottom under the first centring.
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
            all_pending: false,
            position: Position::default(),
            sideways: Bar::new(),
            overflow: false,
            said: u32::MAX,
            reveal_again: None,
        }
    }

    /// The view's key, `code` unless set: its rows are keyed `{key}-l{row}`, each row's text
    /// `t` and its number `n`. Give each code view on screen its own.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.content = Rc::new(Content::new(key.into(), self.content.rows.text.clone()));
        self
    }

    /// Colours the text through `highlighter`, detecting its grammar from `file_name` (and its
    /// first line), or using `syntax` (a token such as `rs`, or a grammar's name).
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

    /// Replaces the text (a reload, or a source that grew while loading). An open find runs
    /// again on the new text, and the next step selects a match; colours start over.
    pub fn set_text(&mut self, text: impl Into<Arc<str>>) {
        self.content = Rc::new(Content::new(self.content.key.clone(), text.into()));
        self.revision = serial();
        self.all_pending = false;
        self.folded = OnceCell::new();
        self.found = self.run_find();
        self.current = None;
        self.pending.retain(|r| *r == Request::Clear);
        self.position.reset();
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
    }

    pub(crate) fn line_numbers(&self) -> bool {
        self.line_numbers
    }

    /// Lines shown above the text: "Preview limited to the first 1 MB", or why a document
    /// fell back to its source.
    pub fn set_notices(&mut self, notices: Vec<String>) {
        self.notices = notices;
    }

    pub(crate) fn notices(&self) -> &[String] {
        &self.notices
    }

    /// `Some(DocumentView::Source)` for a document's source (Markdown, HTML, RTF, CSV, TSV):
    /// the header then offers View rendered, and the host swaps the view.
    pub fn set_document_view(&mut self, form: Option<DocumentView>) {
        self.document_view = form;
    }

    pub(crate) fn content(&self) -> Rc<Content> {
        self.content.clone()
    }

    pub(crate) fn rows(&self) -> &Rows {
        &self.content.rows
    }

    /// The viewport's vertical offset, as Rune last reported it.
    pub fn scroll_offset(&self) -> f32 {
        self.position.offset()
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

    pub(crate) fn scroll_key(&self) -> String {
        format!("{}-scroll", self.content.key)
    }

    /// Rune's visible rows: asks for the colours of the lines they show.
    pub(crate) fn set_visible(&mut self, rows: Range<usize>) {
        self.visible = rows.clone();
        self.position.set_visible(rows.clone());
        let lines = self.content.rows.lines_of(rows);
        if let Some(doc) = &mut self.doc {
            doc.show(lines);
        }
    }

    /// Takes the colours that have landed and applies what find asked for: call it from the
    /// host's poll, which has a `Tx` ([`HostedPreview::find`] gets none). True when the view
    /// changed.
    pub fn poll(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.doc.as_mut().is_some_and(CodeDoc::poll);
        if let Some(row) = self.reveal_again.take() {
            tx.scroll_center_in(self.list_key(), format!("{row}/current"));
        }
        for r in std::mem::take(&mut self.pending) {
            changed = true;
            match r {
                Request::Select(key, range) => tx.select_text(key, range),
                Request::Clear => tx.clear_text_selection(),
                Request::Reveal(row) => {
                    tx.scroll_center_in(self.list_key(), format!("{row}/current"));
                    self.reveal_again = Some(row);
                }
            }
        }
        let overflow = self.position.overflows();
        if overflow != self.overflow {
            self.overflow = overflow;
            changed = true;
        }
        // The bar's accessible value follows the scroll in two-hundredths, so a scroll runs
        // the view at most 200 times from end to end.
        let said = (self.position.progress() * 200.0).round() as u32;
        if said != self.said || self.position.moved_by_step() {
            self.said = said;
            changed = true;
        }
        changed
    }

    /// Asks for the colours of `lines` and blocks until the highlighter has answered, or
    /// `timeout` passes. For headless snapshots and tests only: never call it while building
    /// or polling a window.
    pub fn settle(&mut self, lines: Range<usize>, timeout: std::time::Duration) -> bool {
        match &mut self.doc {
            Some(doc) => {
                doc.show(lines);
                doc.settle(timeout)
            }
            None => true,
        }
    }

    /// Ctrl+A, with the selection as it stood at the key (C11's rule: see
    /// `DocumentState::select_all`).
    pub(crate) fn select_all(&mut self, before: Option<&TextSelection>) {
        let already = before.is_some_and(|sel| copy::reads_whole(&self.content, sel));
        self.content.all.set(already);
        self.all_pending = !already;
    }

    pub(crate) fn all_selected(&self) -> bool {
        self.content.all.get()
    }

    /// The selection changed: the change Ctrl+A's select-all makes starts copy-everything if
    /// it reads as the whole text; any other change ends it.
    pub(crate) fn selection_changed(&mut self, sel: Option<&TextSelection>) {
        let pending = std::mem::take(&mut self.all_pending);
        let all = pending && sel.is_some_and(|sel| copy::reads_whole(&self.content, sel));
        self.content.all.set(all);
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

    /// Selects match `current` and centres it. A match that runs past its row (a line split
    /// at 2 KiB) selects only its part in the first row (Rune selects within one text: R1).
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
        self.content.all.set(false);
        self.all_pending = false;
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
