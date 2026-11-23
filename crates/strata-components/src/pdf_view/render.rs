// SPDX-License-Identifier: MIT

use std::rc::Rc;
use std::time::Duration;

use rune::{
    Color, Cursor, Drag, El, GrowthSpec, Key, KeyEvent, PointerEvent, PointerPress, ScrollSpec,
    SurfaceFit, Tx, WheelEvent, col, el,
};
use rune_kit::Spinner;
use strata_kit::look::Look;

use super::model::{Content, Shared};
use super::queue::Slot;
use super::state::{Gesture, PdfState};
use super::zoom::GAP;
use super::{PdfIntent, PdfLabels};
use crate::contract::Point;
use crate::dialog::FOCUS_RING_WIDTH;
use crate::scroll::{BarLook, overlay};

pub(crate) type IntentFn<S> = Rc<dyn Fn(&mut S, PdfIntent, &mut Tx)>;

/// What a row's builder and the handlers share.
pub(crate) struct Ctx<S> {
    pub get: fn(&S) -> &PdfState,
    pub get_mut: fn(&mut S) -> &mut PdfState,
    pub labels: Rc<PdfLabels>,
    pub intent: Option<IntentFn<S>>,
    pub letters: bool,
    pub reset: fn(&KeyEvent) -> bool,
    pub paint: Paint,
    pub kit: rune_kit::Kit,
}

/// The colours the view draws with: Kit and strata-kit tokens only.
#[derive(Clone, Copy)]
pub(crate) struct Paint {
    pub pane: Color,
    pub paper: Color,
    pub edge: Color,
    pub selection: Color,
    pub hit: Color,
    pub active_hit_border: Color,
    pub spinner: Color,
    pub focus: Color,
}

impl Paint {
    pub fn from_look(look: &Look) -> Paint {
        let k = &look.kit;
        Paint {
            pane: if look.transparency.enabled {
                Color::TRANSPARENT
            } else {
                k.c.bg_base
            },
            paper: look.transparency.opaque(k.c.layer_01),
            edge: k.c.border_muted,
            selection: k.selection(),
            hit: k.alpha(look.fill.highlight, 0.35),
            active_hit_border: k.alpha(k.c.border_focus, 0.85),
            spinner: k.c.icon_muted,
            focus: k.c.border_focus,
        }
    }
}

pub(crate) fn pages<S: 'static>(ctx: Rc<Ctx<S>>, st: &PdfState, look: &Look, scale: f32) -> El {
    let key = st.key().to_owned();
    let scroll_key = st.scroll_key();
    let get_mut = ctx.get_mut;
    let get = ctx.get;
    let labels = &ctx.labels;
    let n = st.document().pages();
    // No rows until the viewport's width is known: they'd lay out at a guess, then again.
    let rows = if st.viewport.is_some() { n } else { 0 };
    let rows_ctx = ctx.clone();
    let list = col()
        .key(st.list_key())
        .w_full()
        .min_w(st.content_width())
        .min_h(st.viewport.map_or(0.0, |v| v[3]))
        .growth(GrowthSpec::default())
        .virtual_rows(
            rows,
            st.row_estimate(),
            move |s: &S, i| get(s).row_key(i).into(),
            move |s: &S, i| page_row(&rows_ctx, get(s), i),
        )
        .virtual_rows_version((st.queue.generation, st.zoom_gen))
        .on_visible_rows(move |s: &mut S, r| get_mut(s).set_visible(r))
        // Ctrl+wheel over the pages: a wheel goes no further out than its scroll viewport.
        .on_wheel(move |s: &mut S, e: WheelEvent, tx: &mut Tx| wheel(get_mut(s), e, tx));
    let feed = st.feed.clone();
    let (bar, sideways) = (st.bar.clone(), st.sideways.clone());
    let viewport = col()
        .key(scroll_key.clone())
        .w_full()
        .h_full()
        .fill(ctx.paint.pane)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_xy()
        .on_scroll_offset(move |p| {
            feed.set(p);
            bar.follow(p);
            sideways.follow(p);
        })
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).set_viewport(r, scale, tx) {
                tx.live_only();
            }
        })
        .child(list);
    let bars = [
        st.bar
            .build::<S>(&format!("{key}-bar"), &scroll_key, BarLook::app(look)),
        st.sideways.build::<S>(
            &format!("{key}-sideways"),
            &scroll_key,
            BarLook::app(look).horizontal(),
        ),
    ];
    let body = st.bar.area(st.sideways.area(overlay(viewport, bars)));
    let cursor = match st.gesture {
        Gesture::Pan(_) => Cursor::Grabbing,
        _ if st.over_text => Cursor::Text,
        _ => Cursor::Grab,
    };
    let position = labels
        .position
        .replace("{page}", &(st.current + 1).to_string())
        .replace("{pages}", &n.to_string())
        .replace("{zoom}", &format!("{:.0}", st.level() * 100.0));
    let keys_ctx = ctx.clone();
    let menu_intent = ctx.intent.clone();
    let mut root = col()
        .key(key.clone())
        .role("document")
        .label(labels.document.clone())
        .a11y_value(position)
        .w_full()
        .h_full()
        .min_h(0.0)
        .min_w(0.0)
        .focusable(true)
        // Inside the edge: the drawer's slot clips anything outside.
        .focus_ring(FOCUS_RING_WIDTH, -FOCUS_RING_WIDTH, ctx.paint.focus)
        .cursor(cursor)
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| key_in(&keys_ctx, s, ev, tx))
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let shift = tx.mods().shift;
            if !get_mut(s).drag(d, shift, tx) {
                tx.live_only();
            }
        })
        // Over the bars, which sit outside the scroll viewport.
        .on_wheel(move |s: &mut S, e: WheelEvent, tx: &mut Tx| wheel(get_mut(s), e, tx))
        .on_pointer_move_local(move |s: &mut S, e: PointerEvent, tx: &mut Tx| {
            if !get_mut(s).hover([e.window_x, e.window_y]) {
                tx.live_only();
            }
        })
        .on_hover_change(move |s: &mut S, inside: bool, tx: &mut Tx| {
            if !inside {
                get_mut(s).leave();
            }
            tx.live_only();
        })
        .child(body);
    if let Some(emit) = menu_intent {
        root = root.on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
            emit(s, PdfIntent::ContextMenu(Point::new(p.x, p.y)), tx);
        });
    }
    if st.wake > 0 {
        root = root.child(
            el().key(format!("{key}-wake-{}", st.wake))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(0.0, 0.0)
                .a11y_hidden()
                .on_layout_tx(move |s: &mut S, _r: [f32; 4], tx: &mut Tx| {
                    if !get_mut(s).apply_level_request(tx) {
                        tx.live_only();
                    }
                    tx.wake_after(Duration::ZERO);
                }),
        );
    }
    if st.viewport.is_some() && st.scale != scale {
        root = root.child(
            el().key(format!("{key}-rescale-{:08x}", scale.to_bits()))
                .pin(Some(0.0), Some(0.0), None, None)
                .size(0.0, 0.0)
                .a11y_hidden()
                .on_layout_tx(move |s: &mut S, _r: [f32; 4], tx: &mut Tx| {
                    let st = get_mut(s);
                    if let Some(vp) = st.viewport {
                        st.set_viewport(vp, scale, tx);
                    }
                    let vis = st.visible();
                    st.set_visible(vis);
                    tx.wake_after(Duration::ZERO);
                }),
        );
    }
    root
}

fn wheel(st: &mut PdfState, e: WheelEvent, tx: &mut Tx) -> bool {
    // Any Ctrl, as Strata's PDF zoom takes it (`install_zoom`): the text size never sees it.
    if !e.mods.ctrl {
        st.anchor = None;
        return false;
    }
    if st.content != Content::Pages {
        return false;
    }
    st.zoom_wheel(e.dy, tx);
    true
}

/// One page's row: the gap above it and the page, centred.
fn page_row<S: 'static>(ctx: &Rc<Ctx<S>>, st: &PdfState, i: usize) -> El {
    let p = &ctx.paint;
    let labels = &ctx.labels;
    let w = st.page_width();
    let h = st.page_height(i);
    let n = st.document().pages();
    let slot = &st.queue.slots[i];
    let get_mut = ctx.get_mut;
    let zoom_gen = st.zoom_gen;
    let mut page = el()
        .key("page")
        .role("group")
        .label(
            labels
                .page
                .replace("{page}", &(i + 1).to_string())
                .replace("{pages}", &n.to_string()),
        )
        .size(w, h)
        .fill(p.paper)
        .outline(1.0, 0.0, p.edge)
        .center()
        .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).page_laid_out(i, zoom_gen, r) {
                tx.live_only();
            }
        });
    page = page_body(ctx, page, slot, [w, h]);
    if let Some(layer) = &slot.layer {
        page = page.children(layer_children(ctx, st, (i, layer), [w, h]));
    }
    if !labels.page_text.is_empty() {
        page = page.child(
            el().key("text-area")
                .role("image")
                .label(labels.page_text.clone())
                .pin(Some(0.0), Some(0.0), None, None)
                .size(w, h),
        );
    }
    col().w_full().h(h + GAP).pt(GAP).items_center().child(page)
}

/// The page's image once it holds one, else its spinner or its failure.
fn page_body<S>(ctx: &Ctx<S>, page: El, slot: &Slot, [w, h]: [f32; 2]) -> El {
    let labels = &ctx.labels;
    match (&slot.pixels, slot.held) {
        (Some(px), Some(_)) => page.child(
            el().key("image")
                .pin(Some(0.0), Some(0.0), None, None)
                .size(w, h)
                .pixels_fit(px, SurfaceFit::Fill)
                .a11y_hidden(),
        ),
        _ if slot.failed => page.description(labels.failed_page.clone()),
        _ => page.description(labels.loading.clone()).child(
            Spinner::new()
                .color(ctx.paint.spinner)
                .build(&ctx.kit)
                .a11y_hidden(),
        ),
    }
}

/// Page `i`'s find hits and its selection as bands over `layer`, then its text.
fn layer_children<S>(
    ctx: &Ctx<S>,
    st: &PdfState,
    (i, layer): (usize, &Shared),
    [w, h]: [f32; 2],
) -> Vec<El> {
    let p = &ctx.paint;
    let k = w / layer.width;
    let band = |r: [f32; 4], fill: Color, key: String| {
        let (x, y, bw, bh) = (r[0] * k, r[1] * k, (r[2] - r[0]) * k, (r[3] - r[1]) * k);
        let pad = bh * 0.06;
        el().key(key)
            .role("presentation")
            .pin(Some(x - pad), Some(y - pad), None, None)
            .size(bw + 2.0 * pad, bh + 2.0 * pad)
            .radius((bh * 0.16).min(3.0))
            .fill(fill)
    };
    let band = &band;
    let selected_range = st.selection.ranges.get(&i).copied();
    let active_hit = st
        .find
        .current_hit()
        .filter(|h| h.page == i)
        .map(|h| (h.start, h.end));
    let hits = st
        .hits_on(i)
        .iter()
        .enumerate()
        // Avoid double-painting a hit band underneath the active match's selection band.
        .filter(move |(_, hit)| selected_range != Some((hit.start, hit.end)))
        .flat_map(|(m, hit)| {
            let runs = layer.selection_runs(hit.start, hit.end).into_iter();
            runs.enumerate()
                .map(move |(j, r)| band(r, p.hit, format!("hit-{m}-{j}")))
        });
    let selected = selected_range.into_iter().flat_map(|(start, end)| {
        let active = active_hit == Some((start, end));
        let runs = layer.selection_runs(start, end).into_iter();
        runs.enumerate().map(move |(j, r)| {
            let mut el = band(r, p.selection, format!("sel-{j}"));
            if active {
                el = el.outline(1.0, 0.0, p.active_hit_border);
            }
            el
        })
    });
    let text = layer.has_text().then(|| {
        el().key("text")
            .role("paragraph")
            .label(layer.text.clone())
            .pin(Some(0.0), Some(0.0), None, None)
            .size(w, h)
    });
    hits.chain(selected).chain(text).collect()
}

/// The view's keys, with focus in it.
fn key_in<S: 'static>(ctx: &Ctx<S>, s: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    let st = (ctx.get_mut)(s);
    if (ctx.reset)(ev) {
        // Consumed even at fitted width: Strata's text size never hears it.
        st.reset_zoom(tx);
        return true;
    }
    // Plain chords only: with Shift or Alt added they're the app's.
    let plain = !m.shift && !m.alt;
    if ev.is_command('a') && plain {
        return st.select_all();
    }
    if ev.is_command('c') && plain {
        let text = st.selected_text();
        // Nothing selected: the key goes on, as Strata's `copy_selection` returns false.
        if text.is_empty() {
            return false;
        }
        tx.copy(text.clone());
        if let Some(f) = &ctx.intent {
            f(s, PdfIntent::Copied(text.into()), tx);
        }
        return true;
    }
    if m.cmd || m.ctrl || m.alt || m.logo {
        return match ev.key {
            Key::Home | Key::End if !m.shift && !m.alt => {
                st.anchor = None;
                let y = if ev.key == Key::Home { 0.0 } else { f32::MAX };
                tx.scroll_to_y(st.scroll_key(), y);
                true
            }
            _ => false,
        };
    }
    let step = 40.0;
    let off = st.feed.get().offset;
    match ev.key {
        Key::Up | Key::Down if !m.shift => {
            st.anchor = None;
            let d = if ev.key == Key::Up { -step } else { step };
            tx.scroll_to_y(st.scroll_key(), (off[1] + d).max(0.0));
            true
        }
        Key::PageUp | Key::PageDown => {
            st.anchor = None;
            tx.scroll_page(st.scroll_key(), if ev.key == Key::PageUp { -1 } else { 1 });
            true
        }
        Key::Home | Key::End if !m.shift => {
            st.anchor = None;
            let y = if ev.key == Key::Home { 0.0 } else { f32::MAX };
            tx.scroll_to_y(st.scroll_key(), y);
            true
        }
        Key::Right if !m.shift && st.can_pan_right() => {
            st.anchor = None;
            tx.scroll_to_x(st.scroll_key(), off[0] + step);
            true
        }
        Key::Left if !m.shift && st.can_pan_left() => {
            st.anchor = None;
            tx.scroll_to_x(st.scroll_key(), (off[0] - step).max(0.0));
            true
        }
        Key::Left if !m.shift => match &ctx.intent {
            Some(f) => {
                f(s, PdfIntent::Leave, tx);
                true
            }
            None => false,
        },
        _ => false,
    }
}
