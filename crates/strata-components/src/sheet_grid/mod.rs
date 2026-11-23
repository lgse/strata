// SPDX-License-Identifier: MIT

mod copy;
mod keys;
mod model;
mod range;
mod render;
mod sort;
mod state;
mod widths;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cursor, Cx, El, GrowthSpec, KeyEvent, ScrollSpec, Tx, col, text};
use rune_kit::shell::{TabItem, TabOrientation, TabOverflow, TabStrip};
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;

use crate::scroll::{BarLook, overlay};

pub use model::{CELL_DISPLAY_BYTES, Sheet, TITLE_DISPLAY_CHARS, column_letters};
pub use range::{CellPos, CellRange};
pub use rune_kit::SortDir;
pub use state::{SheetGridState, SheetIntent, SheetLabels};

use render::{Ctx, Paint, build_row, failure, header, placeholder};

type IntentFn<S> = Rc<dyn Fn(&mut S, SheetIntent, &mut Tx)>;

/// The sheet grid: `SheetGrid::new(get, get_mut) … .build(s, look, cx)`.
pub struct SheetGrid<S> {
    get: fn(&S) -> &SheetGridState,
    get_mut: fn(&mut S) -> &mut SheetGridState,
    intent: Option<IntentFn<S>>,
}

impl<S: 'static> SheetGrid<S> {
    pub fn new(get: fn(&S) -> &SheetGridState, get_mut: fn(&mut S) -> &mut SheetGridState) -> Self {
        SheetGrid {
            get,
            get_mut,
            intent: None,
        }
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, SheetIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    pub fn build(self, s: &S, look: &Look, cx: &mut Cx<S>) -> El {
        let SheetGrid {
            get,
            get_mut,
            intent,
        } = self;
        let st = get(s);
        let key = st.key.clone();
        let mut out = col().key(format!("{key}-box")).w_full().h_full().min_h(0.0);
        if let Some(v) = st.view() {
            out = out.child(grid(s, look, cx, get, get_mut, &intent, v));
        }
        if st.sheet_count() >= 2 {
            out = out.child(tabs(st, look, cx, get_mut, &intent));
        }
        out
    }
}

fn grid<S: 'static>(
    s: &S,
    look: &Look,
    cx: &mut Cx<S>,
    get: fn(&S) -> &SheetGridState,
    get_mut: fn(&mut S) -> &mut SheetGridState,
    intent: &Option<IntentFn<S>>,
    v: &state::SheetView,
) -> El {
    let st = get(s);
    let key = st.key.clone();
    let labels = &st.labels;
    let name = if v.name.is_empty() {
        labels.table.clone()
    } else {
        v.name.to_string()
    };
    let on_key = intent.clone();
    let mut root = col()
        .key(key.clone())
        .role("grid")
        .label(name)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .focusable(true)
        .no_focus_ring()
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            match keys::key_in(get_mut(s), ev, tx) {
                keys::Outcome::Ignored => false,
                keys::Outcome::Taken => true,
                keys::Outcome::Load(i) => {
                    if let Some(f) = &on_key {
                        f(s, SheetIntent::Load(i), tx);
                    }
                    true
                }
            }
        });
    if let Some(c) = v.sel.map(|s| s.cursor) {
        root = root.active_descendant(format!("{}-c{}", st.row_key(c.row), c.col));
    }
    if let Some(reason) = &v.failed {
        let retry = intent.clone();
        // No rows to speak of: a group named by its sheet, around the reason and the button.
        return root.role("group").child(failure(
            look,
            &key,
            labels,
            reason,
            move |s: &mut S, tx| {
                if let Some(i) = get_mut(s).retry()
                    && let Some(f) = &retry
                {
                    f(s, SheetIntent::Load(i), tx);
                }
            },
        ));
    }
    let Some(data) = v.data.as_ref() else {
        return root.child(placeholder(look, format!("{key}-loading"), &labels.loading));
    };
    if data.truncated {
        root = root.child(limited_banner(look, &key, labels));
    }
    if v.columns() == 0 {
        return root.child(placeholder(look, format!("{key}-empty"), &labels.empty));
    }

    let paint = Paint::from_look(look);
    let scroll_key = st.scroll_key();
    let gutter = paint.gutter(data.rows.len());
    let width = cx.layout_size(&scroll_key).map(|z| z[0]);
    let fitted = fitted_widths(v, &paint, width.map(|w| w - gutter));
    let widths = v.manual.clone().unwrap_or_else(|| fitted.clone());
    let cols = range::Columns::new(gutter, &widths);
    *st.laid.borrow_mut() = cols.clone();
    *st.fitted.borrow_mut() = fitted;
    st.row_h.set(paint.style.row_height);
    let built = st.built_columns(&cols, width.unwrap_or(1000.0));
    let ctx = Rc::new(Ctx {
        get,
        get_mut,
        paint,
        labels: labels.clone(),
        cols,
        built,
    });

    let list = body_list(st, v, ctx.clone());
    let (feed, vbar, hbar, strip_x) = (
        st.feed.clone(),
        st.vbar.clone(),
        st.hbar.clone(),
        st.strip_x.clone(),
    );
    let viewport = col()
        .key(scroll_key.clone())
        .role("none")
        .w_full()
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_xy()
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).viewport_laid(r) {
                tx.live_only();
            }
        })
        .on_scroll_offset(move |p| {
            feed.set(p);
            vbar.follow(p);
            hbar.follow(p);
            strip_x.set([-p.offset[0], 0.0]);
        })
        // A press below the last row is on empty space: it clears the range.
        .on_drag(move |s: &mut S, d: &mut rune::Drag, tx: &mut Tx| {
            if d.phase != rune::Phase::Down || !get_mut(s).clear_range() {
                tx.live_only();
            }
        })
        .child(list);
    let vbar = st
        .vbar
        .build::<S>(&format!("{key}-vbar"), &scroll_key, BarLook::app(look));
    let hbar = st.hbar.build::<S>(
        &format!("{key}-hbar"),
        &scroll_key,
        BarLook::app(look).horizontal(),
    );
    let body = st.hbar.area(st.vbar.area(overlay(viewport, [vbar, hbar])));
    root.child(header(&ctx, st, v, &st.strip_x))
        .child(col().w_full().grow(1.0).min_h(0.0).child(body))
}

fn limited_banner(look: &Look, key: &str, labels: &SheetLabels) -> El {
    let k = &look.kit;
    text(labels.limited.clone())
        .key(format!("{key}-limited"))
        .wrap()
        .w_full()
        .font(k.ty.text_small)
        .color(k.c.text_muted)
        .px(look.space.md)
        .py(look.space.xs)
}

fn body_list<S: 'static>(st: &SheetGridState, v: &state::SheetView, ctx: Rc<Ctx<S>>) -> El {
    let (get, get_mut, row_h, width) = (
        ctx.get,
        ctx.get_mut,
        ctx.paint.style.row_height,
        ctx.cols.width(),
    );
    col()
        .key(st.list_key())
        .role("none")
        .cursor(Cursor::Cell)
        .w_full()
        .min_w(width)
        .growth(GrowthSpec::default())
        .virtual_rows(
            v.rows(),
            row_h,
            move |s: &S, i| get(s).row_key(i).into(),
            move |s: &S, i| build_row(&ctx, s, i),
        )
        // Keys are by position: one set per sheet and row count, so no commit reads them.
        .virtual_rows_version((st.active, v.rows()))
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).follow_columns(r[0]) {
                tx.live_only();
            }
        })
        .on_drag(move |s: &mut S, d: &mut rune::Drag, tx: &mut Tx| {
            on_body_drag(get_mut(s), d, tx);
        })
}

fn on_body_drag(st: &mut SheetGridState, d: &rune::Drag, tx: &mut Tx) {
    let changed = match d.phase {
        rune::Phase::Down => {
            st.press(d.x, d.y, tx.mods().shift);
            true
        }
        rune::Phase::Move => st.drag_to(d.x, d.y),
        rune::Phase::Up if d.cancelled() => {
            st.release();
            st.clear_range()
        }
        rune::Phase::Up => {
            let moved = st.drag_to(d.x, d.y);
            st.release();
            moved
        }
    };
    if !changed {
        tx.live_only();
    }
}

fn fitted_widths(v: &state::SheetView, paint: &Paint, available: Option<f32>) -> Vec<f32> {
    let size = paint.style.font.size.to_bits();
    let mut wanted = v.wanted.borrow_mut();
    if wanted.as_ref().is_none_or(|(s, _)| *s != size) {
        let data = v.data.as_ref().expect("a laid-out sheet has data");
        let (body, title) = (paint.style.font, paint.style.header_font);
        let w = widths::wanted(data, paint.style.cell_pad, |t, head| {
            rune::text::advance(t, if head { title } else { body })
        });
        *wanted = Some((size, w));
    }
    let want = &wanted.as_ref().expect("measured").1;
    match available {
        Some(a) if a > 0.0 => widths::fit(want, a),
        _ => want.clone(),
    }
}

fn tabs<S: 'static>(
    st: &SheetGridState,
    look: &Look,
    cx: &mut Cx<S>,
    get_mut: fn(&mut S) -> &mut SheetGridState,
    intent: &Option<IntentFn<S>>,
) -> El {
    let key = format!("{}-tabs", st.key);
    let ids: Vec<String> = (0..st.sheet_count())
        .map(|i| format!("{}-sheet{i}", st.key))
        .collect();
    let items = st.sheets.iter().zip(&ids).map(|(v, id)| {
        // No avatar mark: a sheet has no icon of its own (rune-kit draws an empty box).
        TabItem::new(id.as_str(), v.name.to_string())
            .avatar(rune::el().size(0.0, 0.0))
            .closable(false)
    });
    let intent = intent.clone();
    let prefix = format!("{}-sheet", st.key);
    let strip = TabStrip::new(key.clone(), TabOrientation::Horizontal)
        .tabs(items)
        .active(ids.get(st.active).map(String::as_str))
        .viewport_width(cx.layout_size(&key).map(|z| z[0]))
        .overflow(TabOverflow::ShrinkThenScroll {
            min_width: slot::min_width(look),
            max_width: slot::max_width(look),
            pinned_width: slot::min_width(look),
            fade_width: slot::fade_width(look),
        })
        .label(st.labels.sheets.clone())
        .on_select(move |s: &mut S, id: &str, tx: &mut Tx| {
            let Some(i) = id.strip_prefix(&prefix).and_then(|n| n.parse().ok()) else {
                return;
            };
            if get_mut(s).show(i, tx)
                && let Some(f) = &intent
            {
                f(s, SheetIntent::Load(i), tx);
            }
        })
        .build(&look.kit);
    col()
        .w_full()
        .px(look.space.xs)
        .py(look.space.xs)
        .border_sides([1.0, 0.0, 0.0, 0.0], look.kit.c.border_muted)
        .child(strip)
}
