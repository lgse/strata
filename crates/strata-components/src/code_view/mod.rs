//! CodeView (C10b): Strata's source and plain-text previews and its custom-action script
//! editor, replacing `sourceview5`, with the shared highlighter in `highlight` (C10a).
//!
//! Work package: C10b, `docs/work/phase-2-composites/C10b-code-view.md`. Feature map nodes
//! `preview/preview-panel` (Text), `preview/preview-panel/find`,
//! `preview/preview-panel/documents/scrollbar` and `integration/custom-actions`, against
//! Strata's `src/ui/preview/source_view.rs`, `src/ui/virtual_preview` and
//! `src/ui/settings/actions/editor.rs` at `upstream/main` e9a8993b7.
//!
//! ```ignore
//! // State: code: CodeState, drawer: DrawerState, highlighter: Highlighter (one per app).
//! s.code = CodeState::new(text).highlighter(&s.highlighter, Some("main.rs"), None);
//! let view = CodeView::new(|s: &App| &s.code, |s: &mut App| &mut s.code)
//!     .on_intent(|s: &mut App, intent, tx| s.apply(intent, tx))
//!     .build(s, &s.look, cx);
//! PreviewDrawer::new(&s.drawer, |s: &mut App| &mut s.drawer)
//!     .hosted(&s.code, |s: &mut App| &mut s.code)
//!     .content(view)
//!     .build(files, &s.look, cx);
//! // poll: changed |= s.code.poll(tx);
//! ```
//!
//! # What stays in Strata
//!
//! Which files preview as text, the 1 MiB cap and its notice's wording, decoding (UTF-16,
//! line-break normalising), the wrap preference, which form shows (Rendered or Source) and
//! loading the file. The text arrives as data ([`CodeState::set_text`]); nothing here reads
//! files.
//!
//! # How it draws
//!
//! - **Rows.** One virtual row per line (`El::virtual_rows`, keyed `{key}-l{row}`), and a line
//!   over 2 KiB split into rows of at most 2 KiB on character boundaries, as Strata's large
//!   source preview splits it; the later parts' gutter shows "↳". The view costs one key per
//!   row and the rows on screen, so a 50,000-line file opens at once.
//! - **Line numbers** sit in each row's gutter, so they stay level with their line under wrap
//!   and at any text size. With wrap off they stick to the left edge while the code scrolls
//!   sideways.
//! - **Wrap.** On, each line wraps at the pane's width. Off, the whole text scrolls sideways
//!   as one (`scroll_xy` around the list), the list as wide as the widest row: its columns
//!   times the mono face's advance, or as the widest rows measure when a fallback face (CJK)
//!   draws them wider. Rune draws tabs to four-column stops, as Strata's `tab_width(4)`.
//! - **Colours.** C10a's [`CodeDoc`](highlight::CodeDoc), asked for the lines Rune reports
//!   visible. A line draws plain until its colours land; they only colour, so nothing moves
//!   (probe P2, and a test).
//! - **Find.** C9's find bar calls [`HostedPreview`](crate::preview_drawer::HostedPreview):
//!   literal, case-insensitive, non-overlapping matches over the whole text, folded once per
//!   text (C11's folding). Every match is painted with `look.fill.highlight`
//!   (`TextHighlight`, which never reshapes); the current one is also the selection, and is
//!   centred. The selection and scroll wait for [`CodeState::poll`].
//! - **Selection and copy.** Rune's selectable text in one selection group: drag, double-click
//!   for a word, triple-click for a line, Shift-click. Copying takes the text between the
//!   selection's ends from the model, so a split line copies whole.
//!   Ctrl+A then Ctrl+C copies the whole text from the model, past Rune's 10,000-row cap
//!   (C11's rule).
//! - **The position bar** sits beside the text in its own column while the text overflows,
//!   its thumb at the reader's row, not at Rune's estimated offset (`position.rs`). It is a
//!   Tab stop: Strata's bullet has its ends set "from the keyboard or an accessibility tool"
//!   (`preview/preview-panel/documents/scrollbar`), so Home and End go to the exact start and
//!   end, Up and Down step a row, Page Up and Page Down a page, and assistive technology
//!   reads and sets its row. With wrap off, C24's overlay bar scrolls sideways.
//! - **Keys** in the document: Ctrl+A, Ctrl+C after it, Ctrl+Home and Ctrl+End to the ends,
//!   and plain Left back to the listing ([`CodeIntent::Leave`]).
//! - **Accessibility.** The root is a `document` named by [`CodeLabels::document`]; each
//!   mounted row's text is a text node holding the line (Rune exposes mounted rows only), an
//!   empty one named [`CodeLabels::blank`] ("Blank", as Orca reads an empty line); the
//!   gutter is hidden; the position bar is a slider (Rune has no scroll-bar role, U-C10b-3)
//!   reading "Line N of M".
//!
//! # The editor
//!
//! [`CodeEditor`] is the custom-action script editor: Strata's `script_view()` (monospace,
//! line numbers, `tab_width(4)`, `insert_spaces_instead_of_tabs(true)`, `auto_indent(true)`,
//! wrap off), coloured by an editable [`CodeDoc`](highlight::CodeDoc) and re-highlighted on
//! each edit. See [`editor`] for its keys and its measured cost.
//!
//! # Deviations
//!
//! - **D1** (owner item) No caret or keyboard selection in the read-only view: Shift+arrows,
//!   Shift+Home and End, word steps (C11's R2; U-C10b-2: `text_input` has no read-only mode).
//!   Strata's small source view shows a caret. Ctrl+Home and Ctrl+End scroll to the ends.
//! - **D2** One virtual view for every size. Strata shows small or recognised sources in
//!   sourceview and only large plain text in its virtual view; here every source takes the
//!   virtual view's rules (lines over 2 KiB split into rows).
//! - **D3** Every match is highlighted; Strata highlights only the selected one (plan §4).
//! - **D4** A match that runs past a 2 KiB row boundary selects only its part in the first row
//!   (C11's R1).
//! - **D5** The position bar reads as a slider (U-C10b-3).
//! - **D6** A drag or Shift-click selection over more than 10,000 rows copies Rune's cut list
//!   (C11's R4); Ctrl+A copies whole.
//! - **D7** The editor is built on core `text_input`, not rune-kit `TextArea` (plan §4): a
//!   TextArea takes no `on_secondary_press`, so C21's field menu can't attach (U-C21-2), and
//!   it wraps at its width.
//! - **D8** Lines over 16 KiB draw plain (C10a), and colour reaches a split line's rows
//!   only up to there.
//! - **D9** The editor misses its 16.7 ms keystroke budget on a 5,000-line script (see
//!   [`editor`]), from Rune's whole-text shaping (U-C10b-1, U-C10b-5). Owner item.
//! - **D10** Opening a 50,000-line text takes one commit over the 8 ms budget (14 to 19 ms,
//!   below): Rune reads every row's key on a list's first commit (Rune issue #203, as C1e
//!   reports for its 100,000 rows). The jump to the end and scrolling stay within it.
//! - **D11** A triple-click on a row of a line split at 2 KiB selects that row, not the whole
//!   line (Rune's triple-click takes one text's paragraph: U-C10b-6).
//! - **D12** A drag that starts past the end of a line copies from the next line's start,
//!   without the break after the first line: Rune's selection report leaves out a text
//!   whose selected part is empty, so the start can't be told from it (U-C10b-7).
//! - **D13** The position bar's accessible value follows scrolling in steps of a
//!   two-hundredth of the text (so a scroll runs the view at most 200 times), and exactly
//!   after a step from the keyboard or assistive technology.
//! - **D14** With wrap off, a row of a few glyphs that a fallback face draws far wider than
//!   any estimate can still widen the list when it mounts: `﷽` (U+FDFD, one character, about
//!   12 columns) ×150 took review 2's probe from 1,740 to 14,487 pt. Rows are measured in
//!   the order an estimate ranks them (two columns for anything the mono face doesn't draw),
//!   and only the first six; measuring every row of a large text at open would cost what
//!   virtual rows save.
//!
//! # Measured
//!
//! `tests/perf.rs`, perf profile, NVIDIA, two runs at load average 13 to 17 on 24 cores
//! (review round 1), on `tests/fixtures/code/rust-50k.rs` (50,000 lines, 1.8 MB) with the
//! highlighter thread colouring as it scrolls:
//!
//! | | Wrap on | Wrap off |
//! | --- | --- | --- |
//! | First frame (one commit, D10) | 14.0–15.0 ms | 14.5–15.7 ms |
//! | Ctrl+End, the jump and the 10 frames after it | p95 3.0–3.1 ms, none over 8 ms | p95 3.5–4.0 ms, none over 8 ms |
//! | 600 wheel frames at 120 Hz, 48 pt each | p50 0.8–1.1, p95 1.6–1.8, max 2.5 ms, none over 8 ms | p50 0.5–1.1, p95 1.7–1.8, max 3.0 ms, none over 8 ms |
//! | 120 thumb-drag frames over the whole track | p95 7.4–8.3 ms, 1 and 16 over 8 ms | p95 7.8–8.1 ms, 0 and 8 over 8 ms |
//! | Find "fn" (2,210 matches), the first query | 2.0 ms | 2.0–2.1 ms |
//! | Find " " (469,465 matches) | 4.5–7.4 ms | 7.2–7.5 ms |
//! | Ctrl+A then Ctrl+C, from the model | 42–44 ms, 1,847,325 of 1,847,325 bytes | 47 ms |
//!
//! A thumb drag's frames over 8 ms are C24's U-C24-8: a scroll request past the mounted rows
//! runs the view and commits several times in a frame, where the wheel moving as far
//! doesn't. `RUNE_COMMIT_BUDGET_MS=8` reports no commit over budget while scrolling or after
//! the jump. Find runs in the handler that edits the query: one handler of up to 7.5 ms for a
//! query matching nearly every other byte of 1.8 MB, which a keystroke in the find field pays
//! once; splitting it across frames would leave the count wrong in between, so it isn't.
//!
//! # Rune drafts (not filed)
//!
//! - **U-C10b-1** A text's colour-only spans are part of its shaping key
//!   (`text/hash.rs`, `text_hash` hashes each span's start and end), so any change of colours
//!   reshapes the whole text: in a 153 KB, 5,000-line field every highlight landing costs a
//!   full reshape (about 25 ms with a 200-line span window, 160 ms with every line's 27,780
//!   spans; probe P3). Ask: key shaping on the spans that change shaping only (font, weight,
//!   size, italic, mono), and repaint colour spans over the cached layout.
//! - **U-C10b-2** `text_input` has no read-only mode: a read-only view can't have a caret,
//!   Shift+arrow selection or Home and End by line (Strata's source view, GTK's
//!   `editable(false)` with `cursor_visible(true)`). Ask: `text_input(..).read_only()`,
//!   keeping navigation, selection and copy, and refusing typing, paste, cut and IME.
//! - **U-C10b-3** No scroll-bar role: `a11y/roles.rs` maps no `"scrollbar"`, so a scroll bar
//!   with a value reads as a slider. Ask: map ARIA `scrollbar` to AccessKit's `ScrollBar`,
//!   with the range value and steps a slider has.
//! - **U-C10b-4** A multi-line `text_input` always wraps at its width (`text_input` sets
//!   `wrap`; only `single_line` turns it off). A code editor without wrap has to make the
//!   field wider than its widest line inside a sideways viewport. Ask: `El::no_wrap` for a
//!   multi-line field.
//! - **U-C10b-5** A keystroke in a multi-line `text_input` reshapes its whole text: about
//!   19 ms per key in a 153 KB, 5,000-line field with no spans (probe P3, perf profile).
//!   Ask: shape a field by paragraph and reuse the unchanged paragraphs' layouts.
//! - **U-C10b-6** A triple-click selects one text's paragraph, even inside a selection group:
//!   an app that draws one logical line as several texts (a line split for layout, as here
//!   past 2 KiB) can't have the triple-click take the whole line. Repro: a selection group of
//!   two selectable texts, "aaa" and "bbb", side by side in a row; triple-click "aaa":
//!   `Runtime::selected_text()` is "aaa". Ask: let a group mark texts as parts of one
//!   paragraph (or take the paragraph across texts that share a row key), so a triple-click
//!   selects them all.
//! - **U-C10b-7** A selection's report leaves out an end text whose selected part is empty:
//!   a drag that starts past the end of one text and goes on into the next reports only the
//!   texts from the next one on, so `selection_copy` can't tell that it started on the first
//!   and that the break after it belongs to the copy. Repro: a selection group of two
//!   selectable texts "one" and "two" in a column; press right of "one"'s end and drag into
//!   "two": `TextSelection::texts()` holds "two" alone (with C10b's probe, the copy loses the
//!   "\n"). Ask: report the start text with its empty range at its end (`is_selected` false
//!   but its range kept), as the end text's range is.
//! - **U-C10b-8** An element removed while it holds a drag gets no `Phase::Up`, nor a
//!   cancelled one: its `on_drag_any` hears the Down and the moves, then nothing. A bar whose
//!   preview closes mid-drag stays held (here its trough stayed tinted, review round 1). C10b
//!   ends the drag on the next build that finds the bar wasn't laid out. Repro: an element with
//!   `on_drag_any` recording phases, shown while `s.open`; press on it, move, set `s.open =
//!   false`, frame, release: the handler heard Down and Move only. Related: U-C24-4 (a modal
//!   mounted mid-drag drops the capture the same way). Ask: end a removed element's capture
//!   with a cancelled `Up`, as Escape does mid-drag.
//!
//! Shared with C11: R1 (`select_text` within one text), R2 (no caret in selectable text), R4
//! (copy collection cap), R5.

pub mod editor;
pub mod highlight;

mod copy;
mod find;
mod position;
mod render;
mod rows;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cx, El, GrowthSpec, Key, KeyEvent, PointerButton, ScrollSpec, Tx, col, row, text};
use strata_kit::look::Look;

use crate::dialog::FOCUS_RING_WIDTH;
use crate::scroll::{BarLook, overlay};

pub use editor::{CodeEditor, EditorState};
pub use rows::{CHUNK, TAB};
pub use state::CodeState;

use render::{Ctx, Paint, build_row, gutter_width};

/// What the view tells the host.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodeIntent {
    /// Plain Left with focus in the document: Strata's default key map returns focus to the
    /// listing.
    Leave,
}

/// The view's words, for assistive technology.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CodeLabels {
    /// The document's name.
    pub document: String,
    /// The position bar's name.
    pub position: String,
    /// The position bar's value: `{line}` and `{lines}` are replaced.
    pub line_of: String,
    /// An empty line's name. GtkSourceView exposes its buffer as one text, where Orca reads
    /// an empty line as "blank"; each line here is a text of its own, and an empty text
    /// with no name reads as nothing, so it takes this.
    pub blank: String,
}

impl Default for CodeLabels {
    fn default() -> Self {
        CodeLabels {
            document: "Source".into(),
            position: "Document position".into(),
            line_of: "Line {line} of {lines}".into(),
            blank: "Blank".into(),
        }
    }
}

type IntentFn<S> = Rc<dyn Fn(&mut S, CodeIntent, &mut Tx)>;

/// The source view: `CodeView::new(get, get_mut) … .build(s, look, cx)`.
pub struct CodeView<S> {
    get: fn(&S) -> &CodeState,
    get_mut: fn(&mut S) -> &mut CodeState,
    labels: CodeLabels,
    intent: Option<IntentFn<S>>,
}

impl<S: 'static> CodeView<S> {
    /// The view of the [`CodeState`] that `get` and `get_mut` find in the app's state.
    pub fn new(get: fn(&S) -> &CodeState, get_mut: fn(&mut S) -> &mut CodeState) -> Self {
        CodeView {
            get,
            get_mut,
            labels: CodeLabels::default(),
            intent: None,
        }
    }

    pub fn labels(mut self, labels: CodeLabels) -> Self {
        self.labels = labels;
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, CodeIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let CodeView {
            get,
            get_mut,
            labels,
            intent,
        } = self;
        let st = get(s);
        let paint = Paint::from_look(look);
        let key = st.view_key().to_owned();
        let list_key = st.list_key();
        let scroll_key = st.scroll_key();
        let rows = st.rows();
        let wrap = st.wraps();
        let gutter = st
            .line_numbers()
            .then(|| gutter_width(rows.lines(), paint.mono, paint.pad));
        let bar_look = BarLook::app(look);
        // A bar the last frame didn't lay out was taken away (the preview closed, or the text
        // stopped overflowing): Rune sends a removed element no Up, so a drag it held ends
        // here (U-C10b-8).
        if cx.layout_with(&format!("{key}-position"), |_| ()).is_none() {
            st.position.end_drag();
        }
        st.position.configure(
            position::Keys {
                scroll: scroll_key.clone(),
                list: list_key.clone(),
                row_prefix: format!("{key}-l"),
                fixed: (!wrap).then(|| paint.line()),
            },
            rows.len(),
            bar_look,
        );
        let ctx = Rc::new(Ctx {
            get,
            paint,
            gutter,
            wrap,
            blank: labels.blank.clone(),
        });
        let content = st.content();
        let rows_ctx = ctx.clone();
        let mut list = col()
            .key(list_key.clone())
            .w_full()
            .growth(GrowthSpec::default())
            .selection_group(true);
        if !wrap {
            list = list.min_w(unwrapped_width(st, &paint, gutter.unwrap_or(0.0)));
        }
        let list = list
            .virtual_rows(
                rows.len(),
                paint.line(),
                move |s: &S, i| get(s).row_key_ref(i).into(),
                move |s: &S, i| build_row(&rows_ctx, s, i),
            )
            .virtual_rows_version((key.as_str(), st.revision()))
            .on_visible_rows(move |s: &mut S, r| {
                get_mut(s).set_visible(r);
                false
            })
            .selection_copy(move |sel| Some(content.copy(sel)))
            .on_selection_change(move |s: &mut S, change, tx| {
                get_mut(s).selection_changed(change.selection());
                tx.live_only();
            });
        let (pos, sideways) = (st.position.clone(), st.sideways.clone());
        let pos_vp = st.position.clone();
        let viewport = col()
            .key(scroll_key.clone())
            .w_full()
            .h_full()
            .fill(paint.surface)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .on_scroll_offset(move |p| {
                pos.follow(p);
                sideways.follow(p);
            })
            .on_layout_tx(move |_: &mut S, rect, tx: &mut Tx| {
                tx.live_only();
                pos_vp.viewport(rect);
            });
        let viewport = if wrap { viewport } else { viewport.scroll_xy() };
        let viewport = viewport.child(list);
        let body = if wrap {
            overlay(viewport, [])
        } else {
            let bar = st.sideways.build::<S>(
                &format!("{key}-sideways"),
                &scroll_key,
                BarLook::app(look).horizontal(),
            );
            st.sideways.area(overlay(viewport, [bar]))
        };
        let mut body = row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(col().grow(1.0).min_w(0.0).h_full().child(body));
        if st.overflow {
            body = body.child(position_bar(st, &key, &labels, bar_look, &paint, get_mut));
        }
        let mut root = col()
            .key(key)
            .role("document")
            .label(labels.document.clone())
            .w_full()
            .h_full()
            .min_h(0.0)
            .focusable(true)
            // Keyboard focus shows inside the edge: the drawer's slot clips anything outside.
            .focus_ring(FOCUS_RING_WIDTH, -FOCUS_RING_WIDTH, paint.focus)
            .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
                key_in(s, ev, tx, get_mut, &intent)
            });
        for notice in st.notices() {
            root = root.child(
                text(notice.clone())
                    .wrap()
                    .w_full()
                    .font(paint.small)
                    .color(paint.muted)
                    .px(paint.pad)
                    .py(look.space.sm)
                    .selectable(true)
                    .selection_color(paint.selection),
            );
        }
        root.child(body)
    }
}

/// The list's width with wrap off: the widest row's columns in the mono face's advance, or
/// the rows estimated widest as measured, if a fallback face draws them wider (probe P1 and
/// review 1: CJK and emoji rows widen the list when they mount, which this keeps from
/// happening). Rows are ranked by their drawn width's estimate, not their length, so a short
/// row of wide characters is among them.
fn unwrapped_width(st: &CodeState, paint: &Paint, gutter: f32) -> f32 {
    let rows = st.rows();
    let adv = rune::text::advance("0000000000", paint.mono) / 10.0;
    let by_columns = rows.columns as f32 * adv;
    let measured = rows
        .widest
        .iter()
        .map(|&r| rune::text::advance(rows.source(r), paint.mono))
        .fold(0.0, f32::max);
    (gutter + by_columns.max(measured) + 2.0 * paint.pad).ceil() + 1.0
}

/// The position bar: a column beside the text, while it overflows.
fn position_bar<S: 'static>(
    st: &CodeState,
    key: &str,
    labels: &CodeLabels,
    look: BarLook,
    paint: &Paint,
    get_mut: fn(&mut S) -> &mut CodeState,
) -> El {
    let pos = st.position.clone();
    let rows = st.rows();
    let start = pos.start().round() as usize;
    let line = rows
        .get(start.min(rows.len().saturating_sub(1)))
        .map_or(0, |r| r.line);
    let max = (rows.len() as f64 - pos.span()).max(0.0).round();
    let said = pos.start().round().min(max);
    pos.said(said, max);
    let value = labels
        .line_of
        .replace("{line}", &(line + 1).to_string())
        .replace("{lines}", &rows.lines().to_string());
    let thumb = col()
        .key("thumb")
        .absolute(0.0, 0.0)
        .live_rect(pos.thumb_live())
        .radius(look.thumb_radius)
        .fill(look.thumb);
    let trough = col()
        .key("trough")
        .w_full()
        .h_full()
        .radius(look.trough_radius)
        .live_fill(pos.fill_live())
        .child(thumb);
    let (hover, drag, track) = (pos.clone(), pos.clone(), pos.clone());
    col()
        .key(format!("{key}-position"))
        .w(look.thickness)
        .h_full()
        .role("slider")
        .label(labels.position.clone())
        .a11y_value(value)
        .range_value(said, 0.0, max)
        .range_step(1.0)
        .focus_ring(FOCUS_RING_WIDTH, 0.0, paint.focus)
        .on_step(move |s: &mut S, d, tx| {
            let p = get_mut(s).position.clone();
            p.step_said(f64::from(d), tx);
        })
        .on_step_limit(move |s: &mut S, end, tx| {
            let p = get_mut(s).position.clone();
            p.go(if end { f64::INFINITY } else { 0.0 }, tx);
        })
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            // Down and Page Down move toward the end, as a vertical scroll bar's keys do (a
            // slider's own Down steps its value down).
            let m = ev.mods;
            if m.ctrl || m.cmd || m.alt || m.shift {
                return false;
            }
            let p = get_mut(s).position.clone();
            let page = p.span().floor().max(1.0);
            let rows = match ev.key {
                Key::Up => -1.0,
                Key::Down => 1.0,
                Key::PageUp => -page,
                Key::PageDown => page,
                _ => return false,
            };
            p.step(rows, tx);
            true
        })
        .keeps_text_selection()
        .pointer_buttons([PointerButton::Primary])
        .on_layout_tx(move |_: &mut S, rect, tx: &mut Tx| {
            tx.live_only();
            track.track(rect[3]);
        })
        .on_hover_change_any(move |inside, tx| {
            tx.live_only();
            hover.hover(inside);
        })
        .on_drag_any(move |d, tx| {
            tx.live_only();
            drag.drag(d.phase, d.y, tx);
        })
        .child(trough)
}

/// The document's keys, with focus in it: Ctrl+A and Ctrl+C over the whole text, Ctrl+Home
/// and Ctrl+End to its ends, plain Left back to the listing.
fn key_in<S: 'static>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    get_mut: fn(&mut S) -> &mut CodeState,
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
    // Without this Ctrl+C still copies the whole text (`selection_copy` answers from the
    // model), but only after Rune has built up to 10,000 unbuilt rows to collect the
    // selection it then ignores: 59 ms against 42 to 47 ms on the 50,000-line fixture.
    if ev.is_command('c') && plain && st.all_selected() {
        tx.copy(st.copy_all());
        return true;
    }
    let ctrl = m.ctrl || m.cmd;
    match ev.key {
        Key::Left if !ctrl && !m.shift && !m.alt => match intent.clone() {
            Some(f) => {
                f(s, CodeIntent::Leave, tx);
                true
            }
            None => false,
        },
        Key::Home | Key::End if ctrl && !m.shift => {
            let p = st.position.clone();
            p.go(
                if ev.key == Key::End {
                    f64::INFINITY
                } else {
                    0.0
                },
                tx,
            );
            true
        }
        _ => false,
    }
}
