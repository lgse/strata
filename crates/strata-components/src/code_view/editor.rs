//! [`CodeEditor`]: the custom-action script editor (`integration/custom-actions`, Settings →
//! Actions → Script), on core `text_input` with an editable [`CodeDoc`]'s colours.
//!
//! It follows Strata's `script_view()` (`src/ui/settings/actions/editor.rs`):
//! `monospace(true)`, `show_line_numbers(true)`, `tab_width(4)`,
//! `insert_spaces_instead_of_tabs(true)`, `auto_indent(true)` and `WrapMode::None`, with
//! GtkSourceView's defaults for what it leaves unset (`indent-on-tab` true, `indent-width` -1,
//! so the tab width):
//!
//! - **Return** starts the next line with the current line's leading spaces and tabs, up to
//!   the caret (`auto_indent`).
//! - **Tab** types spaces to the next four-column stop; with a selection over more than one
//!   line it indents each of them by four spaces instead (`indent-on-tab`). **Shift+Tab**
//!   takes up to four spaces, or a tab, off the start of each selected line, or the caret's.
//!   Either is one undo step.
//! - **No wrap**: the field is as wide as its widest line inside a sideways viewport (Rune's
//!   multi-line field always wraps at its width, U-C10b-4), and keeps the caret in view on
//!   both axes (probe P4). The line numbers stick to the left.
//! - **Colours**: each edit goes to the [`CodeDoc`] as a [`LineEdit`], which re-highlights
//!   from the edited line until the parse state matches its cache again (C10a). The field
//!   draws the spans of a window of lines around the viewport, moved in 128-line steps, so
//!   scrolling within it doesn't reshape the field (U-C10b-1).
//! - **Ctrl+Tab** and Ctrl+Shift+Tab move focus out, as GtkTextView's do.
//! - **The field menu** is C21's [`FieldMenu`](crate::context_menu::FieldMenu), through
//!   [`CodeEditor::menu`]: a core `text_input` takes `on_secondary_press`, which rune-kit's
//!   `TextArea` doesn't (U-C21-2).
//!
//! # Measured (perf profile, NVIDIA, load average 13 to 37 on 24 cores)
//!
//! `tests/fixtures/code/shell-5k.sh`, 5,000 lines, 153 KB, typing in its middle
//! (`tests/perf.rs`): a keystroke's frame takes p50 24 ms and p95 28 ms, and the frame its
//! colours land in p50 24 ms and p95 24 ms; Rune's commit report puts 95 % of it in text
//! layout. Probe P3 split it: with no colours a keystroke takes p50 19 ms and p95 21 ms, so
//! shaping the whole field is most of it (U-C10b-5), and the colours' second reshape the rest
//! (U-C10b-1). With every line's spans (27,780) rather than the window's, both take about
//! 160 ms. The re-highlight itself costs under 0.4 ms. At 500 lines a keystroke takes about
//! 6 ms with colours. The budget (16.7 ms at p95 on 5,000 lines) is missed: D9, an owner
//! item. A rich field of `Block::Code` paragraphs (real tab stops in a code block) costs
//! 2.5 times as much (probe P5) and isn't needed: Rune draws tabs to four-column stops in a
//! plain field too.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use rune::{
    Align, El, Key, KeyEvent, Mods, PointerPress, ScrollSpec, TextEdit, Tx, col, row, text,
    text_input,
};
use strata_kit::look::Look;

use super::highlight::{CodeDoc, Highlighter, LineEdit, Mode, OVERSCAN, Source};
use super::render::{Paint, gutter_width};
use super::rows::{TAB, columns_of};

/// The focus ring's width: rune-kit's text fields draw theirs 1 pt wide, just inside.
const FIELD_RING: f32 = 1.0;

/// The field draws the spans of lines in windows this many lines long.
const SPAN_PAGE: usize = 128;

/// The script editor's state: the text being edited and its colours. The host keeps one per
/// editor (one per runtime draft, as Strata keeps a buffer per runtime).
pub struct EditorState {
    pub edit: TextEdit,
    doc: CodeDoc,
    key: String,
    /// The viewport's scroll offset and height, from its handlers.
    offset: Rc<Cell<f32>>,
    height: Rc<Cell<f32>>,
    /// The line height the last build drew with.
    line: Rc<Cell<f32>>,
    /// The lines whose colours the field draws.
    window: Range<usize>,
    /// Ctrl+Tab sent a Tab to move focus on: the editor lets that one Tab pass.
    tab_out: bool,
}

impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("key", &self.key)
            .field("bytes", &self.edit.text.len())
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl EditorState {
    /// An editor on `text`, coloured by `highlighter` with the grammar `syntax` (a token such
    /// as `py` or `bash`), else one detected from `file_name` and the first line.
    pub fn new(
        text: impl Into<String>,
        highlighter: &Highlighter,
        file_name: Option<&str>,
        syntax: Option<&str>,
    ) -> Self {
        let text: String = text.into();
        let mut src = Source::new(text.as_str()).mode(Mode::Editable);
        src.file_name = file_name.map(str::to_owned);
        src.syntax = syntax.map(str::to_owned);
        let mut doc = highlighter.open(src);
        let window = 0..SPAN_PAGE.min(doc.line_count());
        doc.show(window.clone());
        EditorState {
            edit: TextEdit::new(text),
            doc,
            key: "script".into(),
            offset: Rc::default(),
            height: Rc::default(),
            line: Rc::new(Cell::new(1.0)),
            window,
            tab_out: false,
        }
    }

    /// The editor's key, `script` unless set: its field is keyed `field`, its viewport
    /// `{key}-scroll` and its gutter `{key}-numbers`.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    pub fn text(&self) -> &str {
        &self.edit.text
    }

    /// The grammar detection picked, once the highlighter has answered.
    pub fn syntax(&self) -> Option<&str> {
        self.doc.syntax()
    }

    /// Replaces the whole script (a recipe applied), as one undo step.
    pub fn set_text(&mut self, text: &str) {
        let len = self.edit.text.len();
        self.edit.undo_group(|e| {
            e.set_selection(0, len);
            e.insert(text);
        });
        self.edited();
    }

    /// Takes the colours that have landed, and moves the span window with the viewport: call
    /// it from the host's poll. True when the view changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.doc.poll();
        let n = self.doc.line_count();
        let shown = self.shown_lines();
        let want = shown.start..(shown.end + 1).min(n);
        if !(self.window.start <= want.start && want.end <= self.window.end) {
            self.window = window_for(want, n);
            self.doc.show(self.window.clone());
            changed = true;
        }
        changed
    }

    /// Blocks until the highlighter has finished every request, or `timeout` passes. For
    /// headless snapshots and tests only.
    pub fn settle(&mut self, timeout: Duration) -> bool {
        self.doc.settle(timeout)
    }

    /// The colour spans the field draws, in bytes of the text.
    pub fn spans(&self) -> Vec<rune::Span> {
        self.doc.text_spans(self.window.clone())
    }

    /// The lines the viewport shows, as of its last scroll report.
    pub fn shown_lines(&self) -> Range<usize> {
        let line = self.line.get().max(1.0);
        let top = (self.offset.get() / line).floor().max(0.0) as usize;
        let shown = (self.height.get() / line).ceil() as usize;
        let n = self.doc.line_count();
        top.min(n)..(top + shown).min(n)
    }

    /// Whether line `line`'s colours may still change after an edit.
    pub fn is_provisional(&self, line: usize) -> bool {
        self.doc.is_provisional(line)
    }

    /// The text changed outside the field's own typing (a field menu's Cut or Paste, an
    /// undo from the host): re-highlight the lines that did. The field's typing calls it.
    pub fn edited(&mut self) {
        let old = self.doc.text().clone();
        let new = self.edit.text.as_str();
        if *old == *new {
            return;
        }
        let edit = LineEdit::diff(&old, new);
        let n = new.split('\n').count();
        if self.window.end > n || self.window.start >= n {
            self.window = window_for(self.window.start.min(n)..self.window.end.min(n), n);
        }
        self.doc.edit(new.into(), edit, self.window.clone());
    }
}

/// The span window holding `lines` and [`OVERSCAN`] lines past them, in [`SPAN_PAGE`] steps.
fn window_for(lines: Range<usize>, n: usize) -> Range<usize> {
    let start = lines.start.saturating_sub(OVERSCAN) / SPAN_PAGE * SPAN_PAGE;
    let end = (lines.end + OVERSCAN).div_ceil(SPAN_PAGE) * SPAN_PAGE;
    start..end.min(n).max(start.min(n))
}

/// Where line `line` starts, from the byte at `at`'s line.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

/// Return: a line break, then the current line's leading whitespace up to the caret.
pub(crate) fn newline(e: &mut TextEdit) {
    let (a, _) = e.range();
    let start = line_start(&e.text, a);
    let indent: String = e.text[start..a]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    e.insert(&format!("\n{indent}"));
}

/// The lines a selection covers, as the byte range from the first one's start to the last
/// one's end. A selection that ends at a line's start leaves that line out, as GtkSourceView
/// does.
fn selected_lines(e: &TextEdit) -> (usize, usize) {
    let (a, b) = e.range();
    let b = if b > a && e.text[..b].ends_with('\n') {
        b - 1
    } else {
        b
    };
    (line_start(&e.text, a), line_end(&e.text, b))
}

/// Replaces bytes `from..to` with each of their lines mapped by `f`, as one undo step, and
/// selects the lines.
fn map_lines(e: &mut TextEdit, (from, to): (usize, usize), f: impl Fn(&str) -> String) {
    let block: Vec<String> = e.text[from..to].split('\n').map(f).collect();
    let block = block.join("\n");
    let len = block.len();
    e.undo_group(|e| {
        e.set_selection(from, to);
        e.insert(&block);
        e.set_selection(from, from + len);
    });
}

/// Tab: spaces to the next stop, or with a selection over lines, each line indented.
pub(crate) fn indent(e: &mut TextEdit) {
    let (a, b) = e.range();
    if e.text[a..b].contains('\n') {
        let lines = selected_lines(e);
        map_lines(e, lines, |l| {
            if l.is_empty() {
                l.to_owned()
            } else {
                format!("{}{l}", " ".repeat(TAB))
            }
        });
        return;
    }
    let start = line_start(&e.text, a);
    let col = columns_of(&e.text[start..a]);
    e.insert(&" ".repeat(TAB - col % TAB));
}

/// Shift+Tab: up to [`TAB`] spaces, or one tab, off the start of each selected line.
pub(crate) fn outdent(e: &mut TextEdit) {
    let (a, b) = e.range();
    let caret = (a == b).then_some(a);
    let lines = selected_lines(e);
    let first_cut = {
        let l = &e.text[lines.0..];
        if l.starts_with('\t') {
            1
        } else {
            l.chars().take(TAB).take_while(|c| *c == ' ').count()
        }
    };
    map_lines(e, lines, |l| {
        if let Some(rest) = l.strip_prefix('\t') {
            return rest.to_owned();
        }
        let n = l.chars().take(TAB).take_while(|c| *c == ' ').count();
        l[n..].to_owned()
    });
    // A caret stays where it was in its line's text.
    if let Some(c) = caret {
        e.set_caret(c - first_cut.min(c - lines.0));
    }
}

type MenuFn<S> = Rc<dyn Fn(&mut S, PointerPress, &mut Tx)>;

/// The script editor: `CodeEditor::new(get, get_mut) … .build(s, look)`.
pub struct CodeEditor<S> {
    get: fn(&S) -> &EditorState,
    get_mut: fn(&mut S) -> &mut EditorState,
    label: String,
    rows: usize,
    menu: Option<MenuFn<S>>,
}

impl<S: 'static> CodeEditor<S> {
    pub fn new(get: fn(&S) -> &EditorState, get_mut: fn(&mut S) -> &mut EditorState) -> Self {
        CodeEditor {
            get,
            get_mut,
            label: "Script".into(),
            rows: 12,
            menu: None,
        }
    }

    /// The field's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The least height, in lines; the editor grows to fill its parent past it.
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows.max(1);
        self
    }

    /// The field's right-click, Menu key and Shift+F10: C21's `FieldMenu::hook`.
    pub fn menu(mut self, f: impl Fn(&mut S, PointerPress, &mut Tx) + 'static) -> Self {
        self.menu = Some(Rc::new(f));
        self
    }

    pub fn build(self, s: &S, look: &Look) -> El {
        let CodeEditor {
            get,
            get_mut,
            label,
            rows,
            menu,
        } = self;
        let st = get(s);
        let paint = Paint::from_look(look);
        let line = paint.line();
        st.line.set(line);
        let text_ref = &st.edit.text;
        let lines = text_ref.split('\n').count();
        let columns = text_ref.split('\n').map(columns_of).max().unwrap_or(0);
        let adv = rune::text::advance("0000000000", paint.mono) / 10.0;
        // A column of room past the widest line, for the caret at its end.
        let width = ((columns + 1) as f32 * adv).ceil();
        let gutter_w = gutter_width(lines, paint.mono, paint.pad);
        let numbers: String = (1..=lines)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut field = text_input(&st.edit, paint.mono)
            .key("field")
            .label(label)
            .color(paint.text)
            .caret(paint.text, paint.selection)
            .selection_color(paint.selection)
            .min_w(width)
            .on_edit(move |s: &mut S| &mut get_mut(s).edit)
            .on_input_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| key_in(get_mut(s), ev, tx))
            .on_input_change(move |s: &mut S, _tx: &mut Tx| get_mut(s).edited())
            .spans(st.spans());
        if let Some(menu) = menu {
            field = field.on_secondary_press(move |s: &mut S, p, tx: &mut Tx| menu(s, p, tx));
        }
        let gutter = text(numbers)
            .font(paint.mono)
            .color(paint.number)
            .align(Align::End)
            .key(format!("{}-numbers", st.key))
            .w(gutter_w)
            .pr(paint.pad)
            .py(paint.pad / 2.0)
            .fill(paint.surface)
            .sticky_left(0.0)
            .a11y_hidden();
        let content = row().items_start().child(gutter).child(
            col()
                .grow(1.0)
                .px(paint.pad)
                .py(paint.pad / 2.0)
                .child(field),
        );
        let (offset, height) = (st.offset.clone(), st.height.clone());
        let viewport = col()
            .key(format!("{}-scroll", st.key))
            .w_full()
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .scroll_xy()
            .on_scroll_offset(move |p| {
                offset.set(p.offset[1]);
                height.set(p.viewport[1]);
            })
            .child(content);
        let k = &look.kit;
        let mut frame = col()
            .key(st.key.clone())
            .w_full()
            .h_full()
            .min_h(rows as f32 * line + paint.pad)
            .fill(paint.surface)
            .radius(look.round(k.radius.sm))
            .clip_soft(look.round(k.radius.sm))
            .child(viewport);
        // rune-kit's text field frame (`TextArea`'s shell): its input elevation (the ring the
        // theme gives fields) at rest, and its focus ring while the field has focus.
        if st.edit.focused {
            frame = frame.outline(FIELD_RING, -FIELD_RING, k.c.border_focus);
        } else {
            for s in k.shadow_layers(k.elevation.input()) {
                frame = frame.box_shadow(s);
            }
        }
        frame
    }
}

/// The editor's own keys, before the field's.
fn key_in(st: &mut EditorState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    if ev.key == Key::Tab && !(m.ctrl || m.cmd) && std::mem::take(&mut st.tab_out) {
        // The Tab Ctrl+Tab sent: Rune moves focus with it. Only an unmodified Tab is it: a
        // second Ctrl+Tab in the same frame comes before the first one's Tab and sends its
        // own (review 2).
        return false;
    }
    // Ctrl+Tab and Ctrl+Shift+Tab move focus on, as they do out of a GtkTextView, whose Tab
    // types. Rune moves focus only on an unmodified Tab, so the chord sends one (C18's
    // precedent), which the editor lets through.
    if ev.key == Key::Tab && (m.ctrl || m.cmd) && !m.alt && !m.logo {
        st.tab_out = true;
        tx.send_key(KeyEvent {
            key: Key::Tab,
            mods: Mods {
                shift: m.shift,
                ..Mods::default()
            },
        });
        return true;
    }
    if m.ctrl || m.cmd || m.alt || m.logo {
        return false;
    }
    let done = match ev.key {
        Key::Enter if !m.shift => {
            newline(&mut st.edit);
            true
        }
        Key::Tab if m.shift => {
            outdent(&mut st.edit);
            true
        }
        Key::Tab => {
            indent(&mut st.edit);
            true
        }
        _ => false,
    };
    if done {
        st.edited();
    }
    done
}
