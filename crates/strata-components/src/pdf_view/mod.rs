//! PdfView (C12): PDF pages from Strata's sandboxed renderer as a scrolling, zoomable and
//! selectable view. The renderer and its protocol are unchanged.
//!
//! Work package: C12, `docs/work/phase-2-composites/C12-pdf-view.md`. Plan §4 C12. Feature map
//! node `preview/preview-panel/pdf`, with the PDF bullets of `preview/preview-panel`,
//! `.../layout`, `preview/quick-preview` and `app/window/key-dispatch`, against Strata's
//! `src/ui/preview/pdf.rs`, `pdf/`, `pdf_text.rs` and `src/sandbox_helper/pdf.rs` at
//! `upstream/main` e9a8993b7.
//!
//! ```ignore
//! // State: pdf: PdfState, drawer: DrawerState. Strata opens a PDF by rendering its first page.
//! s.pdf.open_with(PdfDocument::new(pages, [w as f32, h as f32]), 0, first_reply);
//! let view = PdfView::new(|s: &App| &s.pdf, |s: &mut App| &mut s.pdf)
//!     .open_button(!s.chooser)
//!     .on_intent(|s: &mut App, intent, tx| s.apply(intent, tx))
//!     .build(s, &s.look, cx);
//! PreviewDrawer::new(&s.drawer, |s: &mut App| &mut s.drawer)
//!     .hosted(&s.pdf, |s: &mut App| &mut s.pdf)
//!     .content(view)
//!     .build(files, &s.look, cx);
//! // poll: hand replies over, take new work; tick and flush every frame.
//! for t in s.pdf.take_cancelled() { s.renderer.cancel(t) }
//! for (t, reply) in s.renderer.finished() { changed |= s.pdf.complete(t, reply).commits() }
//! changed |= s.pdf.tick(now) | s.pdf.flush(tx);
//! while s.renderer.idle() { let Some(r) = s.pdf.pop() else { break }; s.renderer.start(r) }
//! ```
//!
//! # What stays in Strata
//!
//! Rendering (in its sandbox: `page:WxH` to a PNG, `page pages` and a text layer), decoding the
//! PNG into RGBA (off the UI thread, as C25's thumbnails), one render at a time across previews
//! (the host pops only when its renderer is free), which files preview, opening the file
//! ([`PdfIntent::Open`]), where focus goes on [`PdfIntent::Leave`], and the key map (a rebound
//! zoom reset is [`PdfView::reset_key`]; 10xer's letters are [`PdfView::letters`]).
//!
//! # How it works
//!
//! - **Pages.** One virtual row per page (`El::virtual_rows` inside a `scroll_xy` viewport, the
//!   list as wide as the pages, as C10b's unwrapped text). A row's height comes from its page's
//!   size before any image arrives: the size the host knows, else the first page's
//!   ([`PdfDocument`]; Strata's renderer says a page's size only when it renders it). A page is
//!   `Pixels`, filled in place from the host's RGBA; a placeholder with a spinner stands in
//!   until then, and "Unable to render this PDF page" describes one that failed.
//! - **Zoom.** Ctrl+wheel (over the pages or the bars) from 1× to 4× the fitted width (the
//!   pane, at most 1280 points, less 6 points each side), `e^0.14` a notch, about the view's
//!   centre; Ctrl+0 (and keypad 0) back to fitted width on the same page, consumed even there.
//!   Pages draw their old image scaled at once; once the zoom has rested 0.15 s
//!   ([`PdfState::tick`]) the visible pages ask again at the new width, within the renderer's
//!   caps ([`RenderCaps`]: 1400 × 1800, 2.5 MP). A page holding an image within 2 % below or
//!   50 % above the width wanted isn't asked again.
//! - **Requests** are pulled: [`PdfState::pop`] ranks the visible pages from the centre out,
//!   then the next page in the scroll direction, then (with find open) pages whose text isn't
//!   known. Replies from an earlier document, or older than the image a page holds, are
//!   dropped; a request handed out for a page no longer wanted goes to
//!   [`PdfState::take_cancelled`]; a failure reported for such a request is dropped too (a
//!   host may report a stopped render that way). A page that failed asks once more when it
//!   comes back into view, as Strata's rows ask again when bound again. Images and text layers
//!   live under byte budgets (192 MiB and 32 MiB, [`PdfState::set_budgets`]), with find open
//!   too; the visible pages and the page either side are never dropped. A page dropped from the
//!   image budget shows its placeholder, and one that kept its image but lost its layer asks
//!   for a `Page` render again once in view, so its layer comes with the image it draws on
//!   (Strata's protocol renders a page and its text together anyway). A selected page's layer
//!   stays, so its text still copies.
//! - **Selection** is the view's own model over the glyph boxes, as Strata's is (`layer.rs`,
//!   `selection.rs`, ported): a drag over text selects, a double-click a word and a
//!   triple-click a line (without its break), dragging on by words or lines after them;
//!   Shift-click extends, across pages; a drag on a margin, a scan or a page still loading
//!   pans. Bands cover each line's box, cut short below the baseline on runs without
//!   descenders (a unit test and the selection scene's PNG). Ctrl+A selects every page whose
//!   layer the view holds and leaves the anchor where it was, as Strata's `select_all` does, so
//!   a Shift-click after it extends from the last press. Ctrl+C copies in page order with the
//!   document's line breaks; a page between the ends with no layer (a scan, or one never laid
//!   out) adds nothing, as in Strata. With nothing selected Ctrl+C copies nothing and the
//!   key goes on (owner ruling 6): Strata's `copy_selection` returns false for empty text, and
//!   `install_keys` then lets the key propagate (`src/ui/preview/pdf.rs`); Ctrl+A goes on the
//!   same way when no page has text.
//! - **Find** (off unless [`PdfState::set_searchable`]): C9's find bar over the layers, with
//!   C11's folding. Matches are painted with `look.fill.highlight`, the current one is the
//!   selection and is centred. While find is open, pages it hasn't counted are asked for their
//!   text ([`RenderKind::TextOnly`]: the host may render them small): once per query for a page
//!   whose layer the budget dropped, and never again for one the renderer says has none (a
//!   scan, or a layer Strata's helper leaves out: glyphs that don't match, or one over its size
//!   cap). Their matches join in page order. The text budget holds: a page's matches stay
//!   counted after its layer is dropped, and its text comes back when it's shown (a match on
//!   such a page is revealed at the page's top until then).
//! - **Locked and failed** ([`PdfState::locked`], [`PdfState::failed`]): "Preview unavailable"
//!   and the message; a locked PDF has an Open button unless [`PdfView::open_button`] is off
//!   (the chooser). See [`message`].
//! - **Keys** with focus in the pages: the zoom reset, Ctrl+A, Ctrl+C, Up and Down by 40
//!   points, Page Up and Page Down, Ctrl+Home and Ctrl+End, Left and Right pan while the pages
//!   overflow sideways, and plain Left at the left edge is [`PdfIntent::Leave`].
//! - **Accessibility.** The root is a `document` named [`PdfLabels::document`], its value
//!   "Page 3 of 12, 150%" (the page under the centre and the zoom). Each mounted page is a
//!   `group` named "Page N of M", described "Loading page" or "Unable to render this PDF page",
//!   holding its text as a `paragraph` once its layer arrives. Rune exposes the mounted rows
//!   only.
//!
//! # Where the pages are
//!
//! Hit tests, the zoom's anchor and find's reveal use the mounted pages' rects as Rune laid
//! them out (each page's `on_layout_tx`), never sums of estimated heights, as Strata maps
//! points through its bound rows. Rune queues those reports for the next frame, after the
//! host's poll and before input: a handler for input sees where the last frame put the pages,
//! but the poll can run before a scroll's reports. So [`PdfState::set_level`], called from a
//! poll, zooms from a fresh element's layout report, which runs after the pages' own. A row
//! built before a zoom can still report after it; its rect is ignored. Selection extends to the
//! nearest point on any page, measured in window points, so pages drawn at different sizes
//! compare fairly (Strata measures in each page's own pixels).
//!
//! A zoom keeps its anchor until its page has laid out again after the scroll that places it:
//! a notch on the next frame zooms about the same point rather than about pages the last
//! notch's estimate put on screen. The
//! estimate for rows not laid out is the mean of the pages' sizes, so a jump far down a mixed
//! document lands near the right page in its first frame.
//!
//! The I-beam follows the text under the pointer, also when the pages scroll under a still
//! pointer (each page's layout checks the last pointer point); a drag keeps its own cursor and
//! the I-beam is worked out again on release.
//!
//! # Tests
//!
//! Unit tests (`tests.rs`) for the layer's geometry, the selection, the queue (ranking,
//! cancelling, stale replies, the rest, the budgets) and find; in-process tests through the
//! gallery (`crates/strata-gallery/tests/c12.rs`) for request widths and the 1280-point cap,
//! the zoom's rest from wake to wake, cancelling on a jump, where zooms land, the budgets, the
//! cursor and a margin pan, a new display scale and a rebound reset key; one script per
//! Behavior bullet in `tests/scripts/c12/`, with two exceptions. Rune's script key names have no
//! keypad keys at the pinned rev, so "With Num Lock on, Ctrl+keypad 0 also resets" has no
//! script: with Num Lock on, keypad 0 arrives as the same logical "0" a script's `"0"` sends,
//! which the reset reads. "In 10xer mode, Ctrl+0 on a focused zoomed PDF also resets its zoom"
//! has none either: the view's reset doesn't depend on the mode (10xer's key map and chords are
//! Strata's), so a script would repeat `ctrl-zero-same-page`. "Highlight bands … stop short
//! below the baseline" is checked by a unit test and the selection scene's PNG golden.
//!
//! # Measured
//!
//! `tests/perf.rs`, perf profile, NVIDIA, two runs at load average 2 to 3: 500 Letter pages with
//! 2,700-character layers, the host rendering in its poll at once, at scale 2 in a 700 × 900
//! window. The first two frames (open, then the rows at the pane's width) 4.4 ms; 600 wheel
//! frames of 48 points at 120 Hz p50 0.01, p95 1.6 to 1.7, max 4.1 to 4.5 ms (96 view runs);
//! 60 Ctrl+wheel notches, one a frame, p95 0.47 ms, max 0.49 ms, no render while zooming and
//! the 2 visible pages once it rests; Ctrl+End and the 10 frames after p95 2.5 to 4.2 ms; a
//! drag selecting across pages as the wheel scrolls under it p95 0.11 ms, max 1.8 to 2.0 ms;
//! find over 500 layers 3.0 to 4.2 ms (36,000 matches); Ctrl+A then Ctrl+C 1.6 to 2.0 ms for
//! 1.26 MB; `pop` with nothing to do 0.02 µs. No frame over 8 ms, and
//! `RUNE_COMMIT_BUDGET_MS=8` reports no commit over it. After review 1, three runs at load
//! average 20 to 35 (other work on the machine): the same figures within 0.6 ms, except the
//! wheel's slowest frames, max 9.4 to 10.9 ms with 4 to 6 of 600 over 8 ms; the code before
//! review 1, run at the same load, gave max 11.1 ms with 6 over, and no commit was over budget.
//!
//! **At scale 2 the caps leave little to zoom into**: a page in a 700-point pane already asks
//! 1,376 pixels, and Strata's caps allow about 1,390 for a Letter page, so a zoom asks nothing
//! sharper and the page magnifies its image. The re-render at the zoom level (D1) shows at
//! scale 1, or in a pane under 700 points.
//!
//! # Differences from the work package
//!
//! - **No password prompt** (owner ruling 1). The work package asks for a prompt on C15 that
//!   hands the password to the host. Strata's renderer can't take a password
//!   (`open_preview_document` passes none to poppler), the protocol stays unchanged, and
//!   Strata shows "This PDF is password protected." with no password field and an Open button
//!   (`preview/preview-panel`). C12 does the same.
//!
//! # Deviations, for the owner
//!
//! - **D1** Pages render again at the zoom level (the work package's own requirement):
//!   Strata renders at the fitted width only and scales the texture. Within the caps.
//! - **D2** A page with no known size lays out at the first page's shape, not Strata's 560-pixel
//!   placeholder height.
//! - **D3** (owner ruling 3) Zoom keeps the view's centre, as Strata; C14a's image viewport
//!   zooms about the pointer.
//! - **D4** (owner ruling 2) Find over PDF pages, off by default: with it on, PDFs are searched,
//!   where `preview/preview-panel/find` says "PDF pages … are not searched". A match lies within
//!   one page.
//! - **D5** Click counting is Rune's (`Drag::clicks`: the double-click interval, 0.5 s on
//!   Linux, and 4 points), not Strata's 450 ms and 6 pixels.
//! - **D6** A zoom on a document of mixed page sizes can draw a frame off before it lands (probe
//!   P1): Rune places a row only once it has laid it out, so the zoom jumps to where the
//!   centre's page would be if every row above were the mean row's height, then that page
//!   places itself. Strata corrects in an idle callback, a frame late, too. Measured on 500
//!   pages, every third landscape (`a_zoom_far_down_a_mixed_document_keeps_the_centre`): one
//!   notch at page 300 draws one frame 0.13 pages off, four notches on consecutive frames at
//!   most 0.40 pages off on the way, at page 30 at most 0.14; each lands within 0.003 of the
//!   page's height of where it was. Before review 1 one notch drew a frame 1.6 pages off, and
//!   four on consecutive frames landed 3 pages off.
//! - **D7** (owner ruling 8) Each page's text is a `paragraph` for assistive technology, and the
//!   root reads the page and zoom; Strata exposes an image named "PDF page text".
//! - **D8** Text layers stay under a budget plus every selected page, so Ctrl+A's "every
//!   loaded page" can take in more pages than Strata's bound rows.
//! - **D9** C24's overlay bars, not scroll bars beside the pages (as C11's D5).
//! - **D10** Up, Down, Page Up, Page Down, Ctrl+Home and Ctrl+End scroll, Left and Right pan:
//!   no bullet names a PDF's scroll keys.
//!
//! # Rune drafts (not filed)
//!
//! - **U-C12-1** Selectable text can't lie over glyph boxes the app places: Rune's selection
//!   runs on text it shapes itself, so a PDF's text layer needs its own selection, copy, and
//!   accessible text, and assistive technology hears none of its selection. Repro: a `text()`
//!   can't be given per-character rects. Ask: `El::positioned_text(text, rects)` taking part in
//!   the selection, copying and the accessible text. (Upstream #353 is about hit-testing
//!   shaped text, not this.)
//! - **U-C12-2** Virtual rows take one height estimate: a document of mixed page sizes places
//!   its unvisited rows wrong, so a zoom far down can't land in one frame, and the scroll extent
//!   drifts until rows are visited (probe P1). Ask: a per-index height hint
//!   (`virtual_rows_heights(|s, i| h)`), used until a row is measured.
//! - **U-C12-3** `scroll_key_to` places only a row laid out: after a commit that resizes every
//!   row far down the list, the row under the centre isn't mounted at the old offset, and the
//!   request does nothing (probe P1, 500 rows, row 300). Ask: place a virtual row by key from
//!   the commit's layout, as `scroll_into_view` reaches an unmounted row.
//! - **U-C12-4** A hoverable child's cursor never shows inside a draggable parent: the cursor
//!   comes from the element a press would go to (`primary_target`), so a text band with
//!   `Cursor::Text` and a hover handler inside the draggable pages shows the parent's grab
//!   (probe P3). Ask: the cursor of the innermost element that sets one under the pointer.
//!
//! Upstream issues met: #163 (each image page keeps a CPU copy, which doubles the pages'
//! memory), C14a's scale-only draft (the rescale element), C25's U-C25-1 (streamed `Pixels`
//! have no mips: a page zoomed out draws its larger image until the new one lands). rune-kit's
//! `image_zoom` (00cca3c7) measures zoom from fit with no fitted-width levels, so the view
//! keeps C14a's zoom.

mod find;
mod layer;
mod message;
mod model;
mod queue;
mod render;
mod selection;
mod state;
pub mod zoom;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, KeyEvent, Tx};
use strata_kit::look::Look;

pub use find::Hit;
pub use model::{
    Content, MAX_PAGES, PageReply, PdfDocument, RenderCaps, RenderKind, RenderRequest, TextLayer,
    Ticket,
};
pub use queue::{IMAGE_BUDGET, Landed, TEXT_BUDGET};
pub use state::PdfState;

use render::{Ctx, IntentFn, Paint};

/// What the view tells the host.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PdfIntent {
    /// The Open button under a password-protected PDF's message: open it in the default
    /// application.
    Open,
    /// Plain Left at the pages' left edge, or on the Open button: Strata hands the keys back to
    /// the listing.
    Leave,
    /// Ctrl+C copied this (the clipboard already has it).
    Copied(Arc<str>),
}

/// The words the view shows and assistive technology reads. Strata passes translated text;
/// the defaults are Strata's English. `{page}`, `{pages}` and `{zoom}` are replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PdfLabels {
    pub document: String,
    pub page: String,
    pub position: String,
    pub loading: String,
    pub failed_page: String,
    pub unavailable: String,
    pub locked: String,
    pub open: String,
}

impl Default for PdfLabels {
    fn default() -> Self {
        PdfLabels {
            document: "PDF document".into(),
            page: "Page {page} of {pages}".into(),
            position: "Page {page} of {pages}, {zoom}%".into(),
            loading: "Loading page".into(),
            failed_page: "Unable to render this PDF page".into(),
            unavailable: "Preview unavailable".into(),
            locked: "This PDF is password protected.".into(),
            open: "Open".into(),
        }
    }
}

/// The view: `PdfView::new(get, get_mut) … .build(s, look, cx)`. It fills its parent.
pub struct PdfView<S> {
    get: fn(&S) -> &PdfState,
    get_mut: fn(&mut S) -> &mut PdfState,
    labels: PdfLabels,
    intent: Option<IntentFn<S>>,
    open_button: bool,
    letters: bool,
    reset: fn(&KeyEvent) -> bool,
}

impl<S: 'static> PdfView<S> {
    /// The view of the [`PdfState`] that `get` and `get_mut` find in the app's state.
    pub fn new(get: fn(&S) -> &PdfState, get_mut: fn(&mut S) -> &mut PdfState) -> Self {
        PdfView {
            get,
            get_mut,
            labels: PdfLabels::default(),
            intent: None,
            open_button: true,
            letters: false,
            reset: default_reset,
        }
    }

    pub fn labels(mut self, labels: PdfLabels) -> Self {
        self.labels = labels;
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, PdfIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    /// Whether a password-protected PDF's message has an Open button (on unless set). The file
    /// chooser shows the message without it.
    pub fn open_button(mut self, on: bool) -> Self {
        self.open_button = on;
        self
    }

    /// 10xer mode's letters on the message: `j` and `l` move to Open, `h` leaves.
    pub fn letters(mut self, on: bool) -> Self {
        self.letters = on;
        self
    }

    /// Which keys reset the zoom (Ctrl+0 and Ctrl+keypad 0 unless set): Strata's
    /// `pdf_zoom_reset` key map entry.
    pub fn reset_key(mut self, f: fn(&KeyEvent) -> bool) -> Self {
        self.reset = f;
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let PdfView {
            get,
            get_mut,
            labels,
            intent,
            open_button,
            letters,
            reset,
        } = self;
        let st = get(s);
        let ctx = Rc::new(Ctx {
            get,
            get_mut,
            labels: Rc::new(labels),
            intent,
            letters,
            reset,
            paint: Paint::from_look(look),
            kit: look.kit.clone(),
        });
        match st.content() {
            Content::Pages => render::pages(ctx, st, look, cx.env().scale),
            Content::Locked => {
                let message = ctx.labels.locked.clone();
                message::build(&ctx, st, &message, open_button, look)
            }
            Content::Failed(m) => message::build(&ctx, st, m, false, look),
        }
    }
}

fn default_reset(ev: &KeyEvent) -> bool {
    crate::viewers::viewport::ctrl_zero_key(ev).is_some()
}
