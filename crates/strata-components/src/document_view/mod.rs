// SPDX-License-Identifier: MIT

mod copy;
mod find;
mod lang;
mod model;
mod render;
mod shown;
mod state;
mod table;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, GrowthSpec, Key, KeyEvent, PointerPress, ScrollSpec, Tx, col, text};
use strata_kit::look::Look;

use crate::code_view::highlight::CodeBlocks;
use crate::contract::Point as ScreenPoint;
use crate::scroll::{BarLook, overlay};

pub(crate) use find::Folded;
pub use find::{Match, Point};
pub use lang::grammar_token;
pub use model::{
    Cell, Document, ListChildKind, Media, Span, SpanStyle, Unit, UnitKind, char_spans,
};
pub use render::{DocumentLabels, HEADING_SCALE};
pub use rune_kit::SortDir;
pub use state::{COPIED_FOR, DOUBLE_CLICK, DocumentState, MediaState};
pub use table::{MAX_WIDTH as TABLE_MAX_WIDTH, MIN_WIDTH as TABLE_MIN_WIDTH};

use render::{Ctx, IntentFn, Paint, build_row};

/// A row's height before Rune measures it.
const ESTIMATED_ROW: f32 = 28.0;

/// What the view tells the host.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DocumentIntent {
    /// A link was clicked: Strata opens `http` and `https` links externally.
    OpenLink(Arc<str>),
    Leave,
    /// Secondary press (or Menu key) inside the document view, in window points.
    ContextMenu(ScreenPoint),
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
            .selection_copy(move |sel| Some(content.copy(sel)));
        let viewport = col()
            .key(scroll_key.clone())
            .w_full()
            .h_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .on_scroll_offset(st.bar.watch())
            .on_background_press_tx(move |s: &mut S, tx: &mut Tx| {
                get_mut(s).clear_all_selected();
                tx.live_only();
            })
            .press_on_down()
            .child(list);
        let bar = st
            .bar
            .build::<S>(&format!("{key}-bar"), &scroll_key, BarLook::app(look));
        let body = st.bar.area(overlay(viewport, [bar]));
        let menu_intent = intent.clone();
        let mut root = col()
            .key(key)
            .role("document")
            .label(labels.document.clone())
            .w_full()
            .h_full()
            .min_h(0.0)
            .focusable(true)
            .no_focus_ring()
            .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
                key_in(s, ev, tx, get_mut, &intent)
            });
        if let Some(emit) = menu_intent {
            root = root.on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
                emit(
                    s,
                    DocumentIntent::ContextMenu(ScreenPoint::new(p.x, p.y)),
                    tx,
                );
            });
        }
        for notice in &st.document().notices {
            root = root.child(
                col()
                    .w_full()
                    .px(look.space.lg)
                    .pt(look.space.md)
                    .on_background_press_tx(move |s: &mut S, tx: &mut Tx| {
                        get_mut(s).clear_all_selected();
                        tx.live_only();
                    })
                    .press_on_down()
                    .child(
                        col()
                            .w_full()
                            .fill(look.kit.c.info.bg)
                            .border(1.0, look.kit.c.info.border)
                            .radius(look.round(5.0))
                            .px(9.0)
                            .py(7.0)
                            .child(
                                text(notice.clone())
                                    .wrap()
                                    .w_full()
                                    .font(paint.small)
                                    .color(paint.text)
                                    .selectable(true)
                                    .selection_color(paint.selection),
                            ),
                    ),
            );
        }
        root.child(body)
    }
}

/// Vertical line-scroll step in points for arrow-key scrolling.
const SCROLL_STEP: f32 = 40.0;

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
        st.select_all_text(tx);
        return true;
    }
    if ev.is_command('c') && plain && st.all_selected() {
        if tx.text_selection().is_some_and(|sel| st.reads_whole(sel)) {
            tx.copy(st.copy_all());
            return true;
        }
        st.clear_all_selected();
    }
    let ctrl = m.ctrl || m.cmd;
    if ev.key == Key::Left && !ctrl && !m.shift && !m.alt {
        return match intent.clone() {
            Some(f) => {
                f(s, DocumentIntent::Leave, tx);
                true
            }
            None => false,
        };
    }
    scroll_key_in(st, ev, tx)
}

fn scroll_key_in(st: &DocumentState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    let ctrl = m.ctrl || m.cmd;
    if m.alt || m.shift {
        return false;
    }
    match ev.key {
        Key::Up | Key::Down if !ctrl => {
            let delta = if ev.key == Key::Up {
                -SCROLL_STEP
            } else {
                SCROLL_STEP
            };
            tx.scroll_to_y(st.scroll_key(), (st.scroll_offset() + delta).max(0.0));
            true
        }
        Key::PageUp | Key::PageDown if !ctrl => {
            tx.scroll_page(st.scroll_key(), if ev.key == Key::PageUp { -1 } else { 1 });
            true
        }
        Key::Home | Key::End => {
            if ctrl {
                let n = st.document().units.len();
                if n > 0 {
                    let i = if ev.key == Key::Home { 0 } else { n - 1 };
                    tx.scroll_into_view(st.row_key(i));
                }
            } else {
                st.scroll_to_edge(ev.key == Key::End, tx);
            }
            true
        }
        _ => false,
    }
}
