//! DocumentView (C11): Strata's rendered document previews (Markdown, the HTML subset, RTF,
//! DOCX, ODT and OTT) drawn natively from Strata's own laid-out model.
//!
//! Work package: C11, `docs/work/phase-2-composites/C11-document-view.md`. Feature map nodes
//! `preview/preview-panel/documents` and `.../odt`, against Strata's `src/services/document`
//! and `src/ui/virtual_preview` at `upstream/main` 35ee677ab (the feature map's watermark,
//! 5738aad, isn't in the local clone; K2 used the same commit). Spreadsheets, sortable and
//! resizable tables and cell ranges are C11b.
//!
//! ```ignore
//! // State: doc: DocumentState, blocks: CodeBlocks (one pool per window), drawer: DrawerState.
//! let view = DocumentView::new(|s: &App| &s.doc, |s: &mut App| &mut s.doc)
//!     .blocks(&s.blocks)
//!     .on_intent(|s: &mut App, intent, tx| s.apply(intent, tx))
//!     .build(s, &s.look, cx);
//! PreviewDrawer::new(&s.drawer, |s: &mut App| &mut s.drawer)
//!     .hosted(&s.doc, |s: &mut App| &mut s.doc)
//!     .content(view)
//!     .build(files, &s.look, cx);
//! // poll: changed |= s.blocks.poll() | s.doc.tick(now) | s.doc.flush(tx);
//! ```
//!
//! # The model and what stays in Strata
//!
//! Strata parses each format, lays the result out into units (splitting blocks over 32 KiB
//! and lines over 2 KiB), applies its limits and fallbacks, decodes images and renders
//! diagrams and equations, all in its sandbox, and converts its `DocumentLayout` into
//! [`Document`]: one [`Unit`] per row. Images arrive as `Pixels`
//! ([`DocumentState::set_media`]); a link click arrives back as an intent. Which form shows
//! (Rendered or Source), the wrap preference and opening links are Strata's too.
//!
//! # How it draws
//!
//! - **Rows.** One virtual row per unit (`El::virtual_rows`), keyed `{key}-u{index}`, inside
//!   one selection group. Rows build only in Rune's mounted window, so a large document costs
//!   one key per unit and the rows on screen.
//! - **Selection and copy.** Rune's selectable text across rows: drag across blocks,
//!   double-click a word, triple-click a line (a paragraph is one line), Shift-click. Copying
//!   follows Strata's rule: units between the ends copy their copy text, so images
//!   and rules between them copy too. Ctrl+A is the view's own (the root takes focus from a
//!   click on its text, and hears the key before the selection does): Rune's selection
//!   collects at most 10,000 unbuilt rows (R4), so after Ctrl+A, Ctrl+C copies the whole
//!   document from the model. That holds from Ctrl+A over a selection that already reads as
//!   the whole document (Rune's select-all then changes nothing), or from the one change
//!   select-all makes if that reads as the whole document, until the next change. "Reads as
//!   the whole document": every selected text whole, from the document's first text to its
//!   last, or to Rune's cap. A change that didn't come from Ctrl+A never turns it on, so where
//!   Rune's report can't tell, Ctrl+C copies what Rune reports, cut short, not more than was
//!   selected.
//! - **Code.** Coloured by C10a's [`CodeBlocks`] (one pool per window, polled by the host),
//!   only when the block is one unit, as Strata does, and only near the viewport: rows Rune
//!   builds elsewhere (for a copy) draw plain, so a copy never floods the highlighter. Copy
//!   code sits over the first line, which keeps room for it with `El::uncopied` figure
//!   spaces; a press shows the check and "Code copied" for 2 s ([`DocumentState::tick`]).
//!   Strata's language ids map to grammars in [`grammar_token`]; Vala has none and draws
//!   plain.
//! - **Find.** The drawer's find bar calls [`HostedPreview`](crate::preview_drawer::HostedPreview): Strata's matcher, over the text
//!   blocks (not tables, images or rules), folded once per document, so a query scans it
//!   without rebuilding it. The selected match becomes the selection through
//!   [`DocumentState::flush`], which the host calls from its poll: the find calls carry no
//!   `Tx`.
//! - **Tables** fit their columns to the text column (the viewport's width at the last
//!   layout, `Cx::layout_with`), shrinking them in proportion down to a floor, then scroll
//!   sideways.
//! - **Links** are keyed `link-{n}` within their text (a URI's slashes would read as a key
//!   path) and open through [`DocumentIntent::OpenLink`] on a plain click: not on Shift, and
//!   once for a double-click, timed by the host's clock against
//!   [`DocumentState::set_double_click_interval`].
//! - **Accessibility.** The root is a `document`; headings carry their level, list items are
//!   `listitem`s with their nesting level, quotes `blockquote`s, code `code`, images `img`
//!   named by their alt text, tables `table`, `row`, `columnheader` and `cell`. Rune exposes
//!   the mounted rows only. List items have no `list` around them: each unit is its own
//!   virtual row, and `El::virtual_rows` takes no element between the list and its rows, so
//!   nothing can group a run of them.
//!
//! # Deviations
//!
//! - **D1** Tables draw static: columns sized from an estimate of their widest text, wrapping
//!   within 320 points; no sorting or resizing (C11b).
//! - **D2** An inline equation shows its source in the mono face: Rune draws no images inside
//!   text (R3). It still copies as `$source$`. (Owner item.)
//! - **D3** No caret or keyboard selection: Shift with the arrows, Home and End, word steps
//!   and Right collapsing a selection (R2). Ctrl+Home and Ctrl+End scroll to the ends.
//!   (Owner item.)
//! - **D4** A find match that spans two units selects only its part in the first (R1).
//! - **D5** C24's overlay bar, not a scroll bar beside the content; the document-position
//!   scroll bar is C10b's.
//! - **D6** Spans are byte ranges: [`Unit::from_char_spans`] converts Strata's.
//! - **D7** With wrap off, each row scrolls sideways on its own: virtual rows lay out at the
//!   viewport's width inside a sideways viewport, so one scroll for the whole document can't
//!   reach the widest row (probe P1, R7).
//! - **D8** Find skips image alt text and rules, which Strata's find reads.
//! - **D9** A double-click or a Shift-click on a link selects nothing (R6). The link opens as
//!   Strata's does: on a plain click, once for a double-click (a second click within the
//!   double-click time doesn't open it again), never on a Shift-click. A drag over a link
//!   selects.
//! - **D10** With wrap off, Copy code stays over the visible start of a first line wider
//!   than the pane; the line's own end, where its room is, is clear once scrolled to.
//!
//! # Measured
//!
//! `tests/perf.rs`, perf profile, NVIDIA, one run: a 1 MiB document of 10,439 units
//! (paragraphs, headings, lists, a Rust block every 25 units, a table every 40) opens in a
//! 6.9 ms first frame; 600 wheel frames at 120 Hz run p50 0.14 ms, p95 0.70 ms, max 0.87 ms,
//! none over `RUNE_COMMIT_BUDGET_MS=8`, with 73 view runs (code coming into reach, colours
//! landing); find takes 3.7 ms for the first query (it builds the folded text) and 0.1 to
//! 0.4 ms after; Ctrl+A then Ctrl+C copies the whole 1,048,673 bytes from the model in
//! 31.7 ms, where Rune's own collection stops at 1,012,857 (R4).
//!
//! # Rune gaps (drafts, not filed)
//!
//! - **R1** (draft, not filed) `Tx::select_text` selects within one text: a range from one
//!   text to another (a find match across two virtual rows) can't be selected from code.
//! - **R2** (draft, not filed) Selectable text has no caret: no Shift+arrow, Shift+Home/End
//!   or word-step extension, no Right collapsing the selection, no grapheme steps.
//! - **R3** (dropped from the filing packet as a duplicate of anomalyco/rune#350) Read-only
//!   rich text holds inline images only as `El::inline_icon`: square, placed before a word and
//!   skipped by selection and copy, so it can't hold an equation image or copy its source.
//! - **R4** (draft, not filed) Copying or reading a selection across unbuilt virtual rows
//!   stops at 10,000 rows or 4 MiB, silently: `selection_copy` sees a cut list, and nothing
//!   says it was cut (probe P3: 10,057 of 12,000 texts). C11 copies Ctrl+A's selection from
//!   its model, but only once select-all's change has come, in the frame after the key: a
//!   Ctrl+C in the same frame as the Ctrl+A copies Rune's report, cut short.
//! - **R5** (draft, not filed) Rune's README says a virtual row scrolling out drops the
//!   selection it holds; the runtime pins both ends instead, and keeps it.
//! - **R6** (draft, not filed) A press on a link inside selectable text is the link's: a
//!   double-click on it selects no word, and a Shift-click on it doesn't extend the selection
//!   (probe P2 and review 1). The handler hears the modifiers (`Tx::mods`), so an app can at
//!   least keep the link shut, as C11 does.
//! - **R7** (draft, not filed) Virtual rows inside a sideways viewport (`scroll_x` around the
//!   vertical viewport, or `scroll_xy`) lay out at the viewport's width: an unwrapped row wider
//!   than it is cut, and the sideways viewport never scrolls, since the list reports no wider
//!   content (probe P1: a 2,000-character row at 788 points in an 800-point viewport, the
//!   sideways offset stuck at 0). A document can't scroll its widest line into view as one.

mod copy;
mod find;
mod lang;
mod model;
mod render;
mod shown;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, GrowthSpec, Key, KeyEvent, ScrollSpec, Tx, col, text};
use strata_kit::look::Look;

use crate::code_view::highlight::CodeBlocks;
use crate::dialog::FOCUS_RING_WIDTH;
use crate::scroll::{BarLook, overlay};

/// The case folding find runs on, shared with C10b's CodeView.
pub(crate) use find::Folded;
pub use find::{Match, Point};
pub use lang::grammar_token;
pub use model::{
    Cell, Document, ListChildKind, Media, Span, SpanStyle, Unit, UnitKind, char_spans,
};
pub use render::{DocumentLabels, HEADING_SCALE};
pub use state::{COPIED_FOR, DOUBLE_CLICK, DocumentState, MediaState};

use render::{Ctx, IntentFn, Paint, build_row};

/// A row's height before Rune measures it.
const ESTIMATED_ROW: f32 = 28.0;

/// What the view tells the host.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DocumentIntent {
    /// A link was clicked: Strata opens `http` and `https` links externally.
    OpenLink(Arc<str>),
    /// Plain Left with focus in the document: Strata's default key map returns focus to the
    /// listing.
    Leave,
}

/// The document view: `DocumentView::new(get, get_mut) … .build(s, look, cx)`.
pub struct DocumentView<S> {
    get: fn(&S) -> &DocumentState,
    get_mut: fn(&mut S) -> &mut DocumentState,
    blocks: Option<CodeBlocks>,
    labels: Rc<DocumentLabels>,
    intent: Option<IntentFn<S>>,
}

impl<S: 'static> DocumentView<S> {
    /// The view of the [`DocumentState`] that `get` and `get_mut` find in the app's state.
    pub fn new(get: fn(&S) -> &DocumentState, get_mut: fn(&mut S) -> &mut DocumentState) -> Self {
        DocumentView {
            get,
            get_mut,
            blocks: None,
            labels: Rc::new(DocumentLabels::default()),
            intent: None,
        }
    }

    /// The window's code-block pool: code blocks colour through it. Without one they draw
    /// plain.
    pub fn blocks(mut self, blocks: &CodeBlocks) -> Self {
        self.blocks = Some(blocks.clone());
        self
    }

    pub fn labels(mut self, labels: DocumentLabels) -> Self {
        self.labels = Rc::new(labels);
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, DocumentIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let DocumentView {
            get,
            get_mut,
            blocks,
            labels,
            intent,
        } = self;
        let st = get(s);
        let paint = Paint::from_look(look);
        let key = st.view_key().to_owned();
        let list_key = st.list_key();
        let scroll_key = st.scroll_key();
        let width = cx.layout_with(&scroll_key, |r| r[2].round());
        let ctx = Rc::new(Ctx {
            get,
            get_mut,
            paint,
            labels: labels.clone(),
            blocks,
            intent: intent.clone(),
            width,
        });
        let content = st.content();
        let rows_ctx = ctx.clone();
        let list = col()
            .key(list_key.clone())
            .w_full()
            .growth(GrowthSpec::default())
            .selection_group(true)
            .virtual_rows(
                st.document().units.len(),
                ESTIMATED_ROW,
                move |s: &S, i| get(s).row_key(i).into(),
                move |s: &S, i| build_row(&rows_ctx, s, i),
            )
            .virtual_rows_version((key.as_str(), st.revision()))
            .on_visible_rows(move |s: &mut S, r| get_mut(s).set_visible(r))
            .selection_copy(move |sel| Some(content.copy(sel)))
            .on_selection_change(move |s: &mut S, change, tx| {
                get_mut(s).selection_changed(change.selection());
                tx.live_only();
            });
        let viewport = col()
            .key(scroll_key.clone())
            .w_full()
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .on_scroll_offset(st.bar.watch())
            .child(list);
        let bar = st
            .bar
            .build::<S>(&format!("{key}-bar"), &scroll_key, BarLook::app(look));
        let body = st.bar.area(overlay(viewport, [bar]));
        let mut root = col()
            .key(key)
            .role("document")
            .label(labels.document.clone())
            .w_full()
            .h_full()
            .min_h(0.0)
            .focusable(true)
            // Keyboard focus shows inside the edge: the drawer's slot clips anything outside.
            .focus_ring(FOCUS_RING_WIDTH, -FOCUS_RING_WIDTH, look.kit.c.border_focus)
            .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
                key_in(s, ev, tx, get_mut, &intent)
            });
        for notice in &st.document().notices {
            root = root.child(
                text(notice.clone())
                    .wrap()
                    .w_full()
                    .font(paint.small)
                    .color(paint.muted)
                    .px(16.0)
                    .pt(8.0)
                    .selectable(true)
                    .selection_color(paint.selection),
            );
        }
        root.child(body)
    }
}

/// The document's keys, with focus in it: Ctrl+A and Ctrl+C over the whole document, plain
/// Left back to the listing, Ctrl+Home and Ctrl+End to its ends.
fn key_in<S: 'static>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    get_mut: fn(&mut S) -> &mut DocumentState,
    intent: &Option<IntentFn<S>>,
) -> bool {
    let m = ev.mods;
    // Rune's plain chord: with Shift or Alt added, Ctrl+A and Ctrl+C are the app's.
    let plain = !m.shift && !m.alt;
    let st = get_mut(s);
    if ev.is_command('a') && plain {
        st.select_all(tx.text_selection());
        tx.select_all_text(st.list_key());
        return true;
    }
    if ev.is_command('c') && plain && st.all_selected() {
        tx.copy(st.copy_all());
        return true;
    }
    let ctrl = m.ctrl || m.cmd;
    match ev.key {
        Key::Left if !ctrl && !m.shift && !m.alt => match intent.clone() {
            Some(f) => {
                f(s, DocumentIntent::Leave, tx);
                true
            }
            None => false,
        },
        Key::Home | Key::End if ctrl && !m.shift => {
            let n = st.document().units.len();
            if n > 0 {
                let i = if ev.key == Key::Home { 0 } else { n - 1 };
                tx.scroll_into_view(st.row_key(i));
            }
            true
        }
        _ => false,
    }
}
