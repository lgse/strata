// SPDX-License-Identifier: MIT

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
use crate::scroll::{Bar, BarLook, BarMode, overlay};

const BAR_INSET: f32 = 2.0;

/// The field draws the spans of lines in windows this many lines long.
const SPAN_PAGE: usize = 128;

fn editor_bar() -> Bar {
    let bar = Bar::new();
    bar.set_mode(BarMode::Always);
    bar
}

pub struct EditorState {
    pub edit: TextEdit,
    doc: CodeDoc,
    key: String,
    /// The viewport's scroll offset and height, from its handlers.
    offset: Rc<Cell<f32>>,
    height: Rc<Cell<f32>>,
    /// The line height the last build drew with.
    line: Rc<Cell<f32>>,
    bar: Bar,
    sideways: Bar,
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
            bar: editor_bar(),
            sideways: editor_bar(),
            window,
            tab_out: false,
        }
    }

    /// Sets when the editor's scroll bars show.
    pub fn set_bar_mode(&self, mode: BarMode) {
        self.bar.set_mode(mode);
        self.sideways.set_mode(mode);
    }

    /// Releases hover and drag state held on the editor's scroll bars when the editor closes.
    pub fn release_bars(&self) {
        self.bar.release();
        self.sideways.release();
    }

    #[cfg(test)]
    pub(crate) fn bars_overflow(&self) -> (bool, bool) {
        (self.bar.overflows(), self.sideways.overflows())
    }

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

fn selected_lines(e: &TextEdit) -> (usize, usize) {
    let (a, b) = e.range();
    let b = if b > a && e.text[..b].ends_with('\n') {
        b - 1
    } else {
        b
    };
    (line_start(&e.text, a), line_end(&e.text, b))
}

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
            .no_focus_ring()
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
            .pt(paint.pad / 2.0)
            .pb(paint.pad)
            .fill(paint.surface)
            .sticky_left(0.0)
            .a11y_hidden();
        let content = row().items_start().child(gutter).child(
            col()
                .grow(1.0)
                .pl(paint.pad)
                .pr(paint.pad + 6.0)
                .pt(paint.pad / 2.0)
                .pb(paint.pad)
                .child(field),
        );
        let body = build_editor_body::<S>(st, content, look);
        let k = &look.kit;
        let mut frame = col()
            .key(st.key.clone())
            .w_full()
            .h_full()
            .min_h(rows as f32 * line + paint.pad)
            .fill(look.transparency.overlay_color(k.c.bg_base))
            .radius(look.round(k.radius.sm))
            .clip_soft(look.round(k.radius.sm))
            .child(body);
        for s in k.shadow_layers(k.elevation.input()) {
            frame = frame.box_shadow(s);
        }
        frame
    }
}

fn build_editor_body<S: 'static>(st: &EditorState, content: El, look: &Look) -> El {
    let scroll_key = format!("{}-scroll", st.key);
    let (offset, height) = (st.offset.clone(), st.height.clone());
    let (bar, sideways) = (st.bar.clone(), st.sideways.clone());
    let viewport = col()
        .key(scroll_key.clone())
        .w_full()
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_xy()
        .on_scroll_offset(move |p| {
            offset.set(p.offset[1]);
            height.set(p.viewport[1]);
            bar.follow(p);
            sideways.follow(p);
        })
        .child(content);
    let vbar = st
        .bar
        .build::<S>(
            &format!("{}-bar", st.key),
            &scroll_key,
            editor_bar_look(look, false),
        )
        .wheel_scrolls(scroll_key.clone());
    let hbar = st
        .sideways
        .build::<S>(
            &format!("{}-sideways", st.key),
            &scroll_key,
            editor_bar_look(look, true),
        )
        .wheel_scrolls(scroll_key);
    st.bar
        .area(st.sideways.area(overlay(viewport, [vbar, hbar])))
}

fn editor_bar_look(look: &Look, horizontal: bool) -> BarLook {
    let mut bar_look = BarLook::app(look);
    if horizontal {
        bar_look = bar_look.horizontal();
    }
    bar_look.edge = BAR_INSET;
    bar_look.ends = BAR_INSET;
    bar_look
}

/// The editor's own keys, before the field's.
fn key_in(st: &mut EditorState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    if ev.key == Key::Tab && !(m.ctrl || m.cmd) && std::mem::take(&mut st.tab_out) {
        return false;
    }
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
