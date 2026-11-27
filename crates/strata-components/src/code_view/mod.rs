// SPDX-License-Identifier: MIT

pub mod editor;
pub mod highlight;

mod caret;
mod copy;
mod find;
mod gutter;
mod position;
mod render;
mod rows;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{
    Color, Cursor, Cx, Drag, El, GrowthSpec, Key, KeyEvent, PointerButton, PointerPress,
    ScrollSpec, Tx, col, el, row, text,
};
use strata_kit::look::Look;

use crate::contract::Point;
use crate::dialog::FOCUS_RING_WIDTH;
use crate::scroll::{BarLook, anchor, overlay};

pub use editor::{CodeEditor, EditorState};
pub use rows::{CHUNK, TAB};
pub use state::CodeState;

use render::{Ctx, Paint, build_row, gutter_width};

/// What the view tells the host.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CodeIntent {
    Leave,
    /// Secondary press (or Menu key) inside the code view, in window points.
    ContextMenu(Point),
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
    document_home_end: bool,
}

impl<S: 'static> CodeView<S> {
    /// The view of the [`CodeState`] that `get` and `get_mut` find in the app's state.
    pub fn new(get: fn(&S) -> &CodeState, get_mut: fn(&mut S) -> &mut CodeState) -> Self {
        CodeView {
            get,
            get_mut,
            labels: CodeLabels::default(),
            intent: None,
            document_home_end: false,
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

    pub fn document_home_end(mut self, on: bool) -> Self {
        self.document_home_end = on;
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let CodeView {
            get,
            get_mut,
            labels,
            intent,
            document_home_end,
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
        st.highlights.set_color(paint.selection);
        if gutter.is_none()
            || cx
                .layout_with(&format!("{key}-gutter-hit"), |_| ())
                .is_none()
        {
            st.stop_gutter_drag();
        }
        let bar_look = BarLook::app(look);
        configure_position_bar(
            st,
            cx,
            (&key, &scroll_key, &list_key),
            (wrap, &paint, bar_look),
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
            .selection_copy(move |sel| Some(content.copy(sel)));
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
            })
            .on_background_press_tx(move |s: &mut S, tx: &mut Tx| {
                get_mut(s).on_pointer_press();
                tx.live_only();
            })
            .press_on_down();
        let viewport = if wrap { viewport } else { viewport.scroll_xy() };
        let viewport = viewport.child(list);
        let body = build_viewport_body::<S>(
            st,
            viewport,
            &ViewportBody {
                key: &key,
                scroll_key: &scroll_key,
                wrap,
                gutter,
                paint: &paint,
                look,
                get_mut,
            },
        );
        let mut body = row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(col().grow(1.0).min_w(0.0).h_full().child(body));
        if st.overflow {
            body = body.child(position_bar(st, &key, &labels, bar_look, &paint, get_mut));
        }
        let menu_intent = intent.clone();
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
                key_in(s, ev, tx, get_mut, &intent, document_home_end)
            });
        if let Some(emit) = menu_intent {
            root = root.on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
                emit(s, CodeIntent::ContextMenu(Point::new(p.x, p.y)), tx);
            });
        }
        for notice in st.notices() {
            root = root.child(
                col()
                    .w_full()
                    .on_background_press_tx(move |s: &mut S, tx: &mut Tx| {
                        get_mut(s).on_pointer_press();
                        tx.live_only();
                    })
                    .press_on_down()
                    .child(
                        text(notice.clone())
                            .wrap()
                            .w_full()
                            .font(paint.small)
                            .color(paint.muted)
                            .px(paint.pad)
                            .py(look.space.sm)
                            .selectable(true)
                            .selection_color(paint.selection),
                    ),
            );
        }
        root.child(body)
    }
}

struct ViewportBody<'a, S> {
    key: &'a str,
    scroll_key: &'a str,
    wrap: bool,
    gutter: Option<f32>,
    paint: &'a Paint,
    look: &'a Look,
    get_mut: fn(&mut S) -> &mut CodeState,
}

fn build_viewport_body<S: 'static>(st: &CodeState, viewport: El, spec: &ViewportBody<'_, S>) -> El {
    let mut area = overlay(viewport, []);
    if let Some(w) = spec.gutter {
        area = area
            .child(gutter_selection_sentinel(spec.key, st.content()))
            .child(gutter_hit(spec, w))
            .child(gutter_divider(spec.key, w, spec.paint));
    }
    if spec.wrap {
        return area;
    }
    let bar = st.sideways.build::<S>(
        &format!("{}-sideways", spec.key),
        spec.scroll_key,
        BarLook::app(spec.look).horizontal(),
    );
    st.sideways.area(area.child(anchor(0.0, 0.0, bar)))
}

fn gutter_selection_sentinel(key: &str, content: Rc<state::Content>) -> El {
    text(" ")
        .key(format!("{key}-gutter-sel"))
        .pin(Some(0.0), Some(0.0), None, None)
        .size(0.0, 0.0)
        .color(Color::TRANSPARENT)
        .selectable(true)
        .selection_color(Color::TRANSPARENT)
        .selection_copy(move |sel| Some(content.copy(sel)))
        .a11y_hidden()
}

fn gutter_hit<S: 'static>(spec: &ViewportBody<'_, S>, w: f32) -> El {
    let get_mut = spec.get_mut;
    el().key(format!("{}-gutter-hit", spec.key))
        .pin(Some(0.0), Some(0.0), None, Some(0.0))
        .w(w + spec.paint.pad)
        .cursor(Cursor::Pointer)
        .wheel_scrolls(spec.scroll_key.to_owned())
        .keeps_text_selection()
        .pointer_buttons([PointerButton::Primary])
        .a11y_hidden()
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            gutter::gutter_drag(get_mut(s), d, tx);
        })
}

fn gutter_divider(key: &str, x: f32, paint: &Paint) -> El {
    el().key(format!("{key}-gutter-divider"))
        .pin(Some(x), Some(0.0), None, Some(0.0))
        .w(1.0)
        .fill(paint.divider)
        .a11y_hidden()
        .inert()
}

fn configure_position_bar<S: 'static>(
    st: &CodeState,
    cx: &mut Cx<S>,
    (key, scroll_key, list_key): (&str, &str, &str),
    (wrap, paint, bar_look): (bool, &Paint, BarLook),
) {
    if cx.layout_with(&format!("{key}-position"), |_| ()).is_none() {
        st.position.end_drag();
    }
    st.position.configure(
        position::Keys {
            scroll: scroll_key.to_owned(),
            list: list_key.to_owned(),
            row_prefix: format!("{key}-l"),
            fixed: (!wrap).then(|| paint.line()),
        },
        st.rows().len(),
        bar_look,
    );
}

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
            p.go_step(if end { f64::INFINITY } else { 0.0 }, tx);
        })
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
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
        .on_secondary_press(|_: &mut S, _, tx: &mut Tx| tx.live_only())
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

/// Vertical line-scroll step in points for arrow-key scrolling.
const SCROLL_STEP: f32 = 40.0;

fn key_in<S: 'static>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    get_mut: fn(&mut S) -> &mut CodeState,
    intent: &Option<IntentFn<S>>,
    document_home_end: bool,
) -> bool {
    let m = ev.mods;
    // Rune's plain chord: with Shift or Alt added, Ctrl+A and Ctrl+C are the app's.
    let plain = !m.shift && !m.alt;
    let st = get_mut(s);
    if ev.is_command('a') && plain {
        st.select_all_text(tx);
        return true;
    }
    if copy_shortcut(st, ev, plain, tx) {
        return true;
    }
    if is_plain_left(ev)
        && let Some(f) = intent.clone()
    {
        f(s, CodeIntent::Leave, tx);
        return true;
    }
    if scroll_key_in(st, ev, document_home_end, tx) {
        return true;
    }
    if caret::handles_key(ev) {
        st.sync_caret_from_selection(tx.text_selection());
        if let Some(handled) = caret::on_key(st, ev, tx) {
            return handled;
        }
    }
    false
}

fn scroll_key_in(st: &CodeState, ev: &KeyEvent, document_home_end: bool, tx: &mut Tx) -> bool {
    let m = ev.mods;
    if m.ctrl || m.cmd || m.shift || m.alt {
        return false;
    }
    match ev.key {
        Key::Up | Key::Down => {
            let delta = if ev.key == Key::Up {
                -SCROLL_STEP
            } else {
                SCROLL_STEP
            };
            tx.scroll_to_y(st.scroll_key(), (st.scroll_offset() + delta).max(0.0));
            true
        }
        Key::PageUp | Key::PageDown => {
            tx.scroll_page(st.scroll_key(), if ev.key == Key::PageUp { -1 } else { 1 });
            true
        }
        Key::Home | Key::End if document_home_end => {
            st.scroll_to_edge(ev.key == Key::End, tx);
            true
        }
        _ => false,
    }
}

fn copy_shortcut(st: &mut CodeState, ev: &KeyEvent, plain: bool, tx: &mut Tx) -> bool {
    if !(ev.is_command('c') && plain) {
        return false;
    }
    if st.all_selected() {
        if tx.text_selection().is_some_and(|sel| st.reads_whole(sel)) {
            tx.copy(st.copy_all());
            return true;
        }
        st.clear_all_selected();
    }
    if let Some(text) = st.copy_keyboard_if_valid(tx.text_selection()) {
        tx.copy(text);
        return true;
    }
    false
}

fn is_plain_left(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    ev.key == Key::Left && !(m.ctrl || m.cmd || m.shift || m.alt)
}
