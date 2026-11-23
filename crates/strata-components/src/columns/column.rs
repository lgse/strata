// SPDX-License-Identifier: MIT

use std::{borrow::Cow, rc::Rc};

use rune::{
    Anim, Color, Cursor, Drag, El, Phase, PointerButton, Spring, SpringPair, Transition, Tx, col,
    el, row, text,
};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Spinner};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::columns as tokens;

use super::fit;
use super::intent::ColumnsIntent;
use super::keys;
use super::state::ColumnsState;
use super::state::Entrance;
use super::view::{HeaderCx, Shared, collection, depth_of};
use super::width::{self, AUTOFIT_WITHIN, CAPTION_DELAY, EDGE_REACH, Resize};
use crate::contract::Rect;

const MOTION: Spring = Spring::new(0.22, 0.0);

/// How far a new column slides in from: Strata's `column-enter` keyframes.
const SLIDE: f32 = -24.0;

pub(super) fn shell<S: 'static>(sh: &Rc<Shared<S>>, s: &S, depth: usize) -> El {
    let p = (sh.parts)(s);
    let st = p.state;
    let look = p.look;
    let c = &st.columns[depth];
    let key = keys::column(&st.key, c.serial);
    let scale = width::scale(look);
    let w = st.drawn_width(depth, c, scale);
    let (shell_motion, body_motion) = shell_motions(look.animations, c.entrance);
    let list = collection(sh, s, c.serial).build(s);
    let body = col()
        .key(format!("{key}-body"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .transition(body_motion)
        .child(c.bar.area(list));
    let n = st.columns.len().min(p.sources.len());
    let draws_line = depth + 1 < n || (sh.trailing_edge)(s);
    let title = if depth < p.sources.len() {
        p.sources.title(depth).to_owned()
    } else {
        String::new()
    };
    let order = n.saturating_sub(depth).min(i16::MAX as usize) as i16;
    let mut shell = col()
        .key(keys::slot(&st.key, c.slot))
        .role("group")
        .label(title.clone())
        .description(sh.labels.pane.clone())
        .w(w)
        .h_full()
        .min_h(0.0)
        .paint_order(order)
        .drawn_rect_into(&c.drawn)
        .transition(shell_motion)
        .child(body);
    if draws_line {
        shell = shell.child(edge_line(&key, look));
    }
    let right = st.spans(look).get(depth).map_or(w, |&(_, r)| r) - st.strip_feed.get().offset[0];
    let overlaps_window_edge = depth + 1 == n
        && draws_line
        && st.strip.w > 0.0
        && (st.strip.w - right).abs() <= EDGE_REACH;
    shell = shell.child(edge(
        sh,
        st,
        EdgeSpec {
            depth,
            serial: c.serial,
            key: &key,
            scale,
            draws_line,
            overlaps_window_edge,
        },
        look,
    ));
    if let Some(slice) = st.clipped_slice(depth, look) {
        shell = shell.child(reveal_target(sh, &title, &key, slice));
    }
    let resizing = st.resize.as_ref().is_some_and(|r| r.depth == depth);
    if st.edge_hover == Some(depth) || resizing {
        let caption = look.resize_caption();
        shell = shell.child(
            text(sh.labels.width.clone())
                .key(keys::caption(&key))
                .role("tooltip")
                .font(look.kit.ty.small)
                .color(caption.ink)
                .px(look.space.sm)
                .py(look.space.xxs)
                .radius(look.round(look.kit.radius.sm))
                .fill(caption.fill)
                .border(tokens::caption_border(look), look.kit.c.border_base)
                .pin(
                    None,
                    Some(tokens::header_height(look) + look.space.sm),
                    Some(EDGE_REACH + look.space.xs),
                    None,
                )
                .paint_order(2),
        );
    }
    if st.drop_column() == Some(depth) {
        shell = shell.child(
            el().key(keys::wash(&key))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(tokens::fade(
                    look,
                    look.kit.c.bg_accent,
                    tokens::drop_opacity(look),
                ))
                .outline(
                    tokens::drop_ring(look),
                    -tokens::drop_ring(look),
                    look.kit.c.bg_accent,
                )
                .a11y_hidden()
                .inert(),
        );
    }
    shell
}

fn shell_motions(animated: bool, entrance: Entrance) -> (Transition, Transition) {
    if !animated {
        return (Transition::none(), Transition::none());
    }
    let pair = SpringPair::new(MOTION, MOTION);
    let shell_in = match entrance {
        Entrance::Slide => Transition::offset(SLIDE, 0.0).combined(Transition::opacity()),
        Entrance::Fade | Entrance::Still => Transition::none(),
    };
    let shell_motion = Transition::asymmetric(shell_in, Transition::opacity())
        .hold_layout()
        .springs(pair);
    let body_motion = if entrance == Entrance::Fade {
        Transition::asymmetric(Transition::opacity(), Transition::none()).springs(pair)
    } else {
        Transition::none()
    };
    (shell_motion, body_motion)
}

struct EdgeSpec<'a> {
    depth: usize,
    serial: u64,
    key: &'a str,
    scale: f32,
    draws_line: bool,
    overlaps_window_edge: bool,
}

fn edge_line(key: &str, look: &Look) -> El {
    el().key(keys::edge_line(key))
        .pin(None, Some(0.0), Some(0.0), Some(0.0))
        .w(tokens::edge_width(look))
        .fill(tokens::fade(
            look,
            look.kit.c.std_border,
            tokens::edge_opacity(look),
        ))
        .transition(Transition::none())
        .a11y_hidden()
        .inert()
}

/// Column resize edge overlay. Uses a dedicated accent line driven by the edge's hover group
/// because Rune lacks hover border colour (`anomalyco/rune#328`).
fn edge<S: 'static>(sh: &Rc<Shared<S>>, st: &ColumnsState, spec: EdgeSpec<'_>, look: &Look) -> El {
    let (drag, lit, hover) = (sh.clone(), sh.clone(), sh.clone());
    let EdgeSpec {
        depth,
        serial,
        key,
        scale,
        draws_line,
        overlaps_window_edge,
    } = spec;
    let line = tokens::edge_width(look);
    let resizing = st.resize.as_ref().is_some_and(|r| r.depth == depth);
    let rest = if resizing { 1.0 } else { 0.0 };
    let caption_hover = el()
        .w_full()
        .h_full()
        .hover_delays(CAPTION_DELAY, std::time::Duration::ZERO)
        .on_hover_change(move |s: &mut S, inside: bool, _tx: &mut Tx| {
            let Some(depth) = depth_of(&hover, s, serial) else {
                return;
            };
            let st = (hover.parts_mut)(s).state;
            if inside {
                st.edge_hover = Some(depth);
            } else if st.edge_hover == Some(depth) {
                st.edge_hover = None;
            }
        });
    let mut handle = el()
        .key(keys::edge(key))
        .pin(None, Some(0.0), Some(-EDGE_REACH), Some(0.0))
        .w(2.0 * EDGE_REACH)
        .cursor(Cursor::ResizeColumn)
        .a11y_hidden()
        .paint_order(1);
    if overlaps_window_edge {
        handle = handle.raise();
    }
    handle = handle
        .hover_group()
        .hover_motion(Anim::INSTANT, Anim::INSTANT)
        .on_hover_change(move |s: &mut S, inside: bool, tx: &mut Tx| {
            let Some(depth) = depth_of(&lit, s, serial) else {
                return;
            };
            let st = (lit.parts_mut)(s).state;
            if inside {
                st.edge_lit = Some(depth);
            } else if st.edge_lit == Some(depth) {
                st.edge_lit = None;
            }
            if depth + 1 != st.columns.len() {
                tx.live_only();
            }
        })
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            resize(&drag, s, serial, d, scale, tx);
        })
        .child(caption_hover);
    if draws_line {
        handle = handle.child(
            el().key(keys::edge_accent(key))
                .pin(Some(EDGE_REACH - line), Some(0.0), None, Some(0.0))
                .w(line)
                .fill(look.kit.c.bg_accent)
                .group_opacity(rest, 1.0)
                .opacity_motion(Anim::INSTANT)
                .transition(Transition::none())
                .a11y_hidden()
                .inert(),
        );
    }
    handle
}

fn resize<S: 'static>(
    sh: &Rc<Shared<S>>,
    s: &mut S,
    serial: u64,
    d: &mut Drag,
    scale: f32,
    tx: &mut Tx,
) {
    if tx.button() != PointerButton::Primary {
        return;
    }
    let Some(depth) = depth_of(sh, s, serial) else {
        return;
    };
    match d.phase {
        Phase::Down => {
            let p = (sh.parts_mut)(s);
            let st = p.state;
            let now = st.now;
            let again = st
                .edge_press
                .is_some_and(|(at, t)| at == serial && now - t <= AUTOFIT_WITHIN);
            st.edge_press = Some((serial, now));
            st.targeted = Some(depth);
            if again {
                st.resize = None;
                autofit(sh, s, depth, scale, tx);
                return;
            }
            // From the width drawn now, so a column narrowed to the area starts there.
            let start = st.drawn_width(depth, &st.columns[depth], scale) / scale;
            st.resize = Some(Resize {
                depth,
                start,
                live: start,
            });
        }
        Phase::Move => {
            let st = (sh.parts_mut)(s).state;
            let cap = st.cap();
            if let Some(r) = st.resize.as_mut().filter(|r| r.depth == depth) {
                tx.instant();
                r.live = width::dragged(r.start, d.dx, scale, cap);
            }
        }
        Phase::Up => {
            let st = (sh.parts_mut)(s).state;
            let Some(r) = st.resize.take().filter(|r| r.depth == depth) else {
                return;
            };
            if d.cancelled() {
                tx.instant();
                return;
            }
            // A click that moved nothing still settles the width under the area, and spreads.
            let w = width::dragged(r.start, d.dx, scale, st.cap());
            st.spread(w);
            tx.motion(MOTION);
            sh.emit(s, ColumnsIntent::Width(w), tx);
        }
    }
}

fn autofit<S: 'static>(sh: &Rc<Shared<S>>, s: &mut S, depth: usize, scale: f32, tx: &mut Tx) {
    let w = {
        let p = (sh.parts)(s);
        let drawn = fit::content_width(p.sources, p.state, p.look, depth);
        width::fitted(drawn, scale, p.state.cap())
    };
    let st = (sh.parts_mut)(s).state;
    st.set_column_width(depth, w);
    tx.motion(MOTION);
    sh.emit(s, ColumnsIntent::Width(w), tx);
}

fn title_label(
    title: String,
    described: Option<String>,
    key: &str,
    look: &Look,
    color: Color,
) -> El {
    let label = text(title.clone())
        .key(keys::title(key))
        .font(tokens::header_title_font(look))
        .color(color)
        .truncate_middle()
        .shrink()
        .grow(1.0)
        .min_w(0.0);
    match described {
        Some(described) => label.role("label").label(title).description(described),
        None => label.a11y_hidden(),
    }
}

pub(super) fn header<S: 'static>(sh: &Rc<Shared<S>>, s: &S, depth: usize, serial: u64) -> El {
    let p = (sh.parts)(s);
    let st = p.state;
    let look = p.look;
    let k = &look.kit;
    let key = keys::column(&st.key, serial);
    let active = depth == st.active;
    let header_active = active && (sh.active_header)(s);
    let targeted = depth == st.targeted();
    let loading = depth < p.sources.len() && p.sources.loading(depth);
    let spinner = depth < p.sources.len() && p.sources.spinner(depth);
    let title = if depth < p.sources.len() {
        p.sources.title(depth).to_owned()
    } else {
        String::new()
    };
    let described = (depth < p.sources.len())
        .then(|| p.sources.title_description(depth))
        .flatten()
        .map(Cow::into_owned);
    let color = if header_active {
        k.c.text_base
    } else {
        tokens::fade(look, k.c.bg_accent, tokens::header_title_opacity(look))
    };
    let mut bar = row()
        .key(keys::header(&key))
        .w_full()
        .h(tokens::header_height(look))
        .pl(tokens::header_inset(look))
        .pr(look.space.xs)
        .gap(look.space.xs)
        .items_center()
        .border_sides(
            [0.0, 0.0, tokens::edge_width(look), 0.0],
            tokens::fade(look, k.c.std_border, tokens::edge_opacity(look)),
        )
        .child(title_label(title, described, &key, look, color));
    if header_active {
        bar = bar.fill(tokens::header_fill(look)).child(
            el().key(keys::indicator(&key))
                .pin(Some(0.0), Some(0.0), Some(0.0), None)
                .h(tokens::header_indicator(look))
                .fill(k.c.bg_accent)
                .a11y_hidden()
                .inert(),
        );
    }
    if spinner {
        bar = bar.child(header_spinner(sh, look, &key));
    }
    let cx = HeaderCx {
        depth,
        active,
        targeted,
        loading,
    };
    if let Some(controls) = header_controls(sh, s, cx, serial) {
        // A secondary press on a control is the control's: no folder menu.
        bar = bar.child(controls.on_secondary_press(|_: &mut S, _, tx: &mut Tx| tx.live_only()));
    }
    let bar = with_header_menu(bar, sh.clone(), serial);
    with_band(sh, s, bar, cx)
}

fn header_spinner<S: 'static>(sh: &Rc<Shared<S>>, look: &Look, key: &str) -> El {
    el().key(keys::spinner(key))
        .role("progressbar")
        .label(sh.labels.loading.clone())
        .child(
            Spinner::new()
                .size(look.icon_size.md)
                .build(&look.kit)
                .a11y_hidden(),
        )
}

fn header_controls<S: 'static>(sh: &Rc<Shared<S>>, s: &S, cx: HeaderCx, serial: u64) -> Option<El> {
    if !cx.targeted {
        return None;
    }
    let p = (sh.parts)(s);
    let look = p.look;
    let key = keys::column(&p.state.key, serial);
    let mut controls = row().gap(look.space.xxs).items_center();
    let mut any = false;
    if let Some(actions) = sh.actions.as_ref().and_then(|f| f(s, cx)) {
        controls = controls.child(actions);
        any = true;
    }
    if cx.depth > 0 && (sh.close_buttons)(s) {
        let close = sh.clone();
        controls = controls.child(
            IconButton::new(look.icon(IconName::Close))
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(sh.labels.close.clone())
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    if let Some(depth) = depth_of(&close, s, serial) {
                        close.emit(s, ColumnsIntent::CloseFrom(depth), tx);
                    }
                })
                .build(&look.accent_icon_kit())
                .focusable(false)
                .no_focus_ring()
                .key(keys::close(&key)),
        );
        any = true;
    }
    any.then_some(controls)
}

fn with_header_menu<S: 'static>(bar: El, menu: Rc<Shared<S>>, serial: u64) -> El {
    bar.on_secondary_press(move |s: &mut S, at, tx: &mut Tx| {
        let Some(depth) = depth_of(&menu, s, serial) else {
            return;
        };
        {
            let st = (menu.parts_mut)(s).state;
            st.targeted = Some(depth);
            if st.active != depth {
                st.set_active_quiet(depth);
            }
        }
        menu.emit(
            s,
            ColumnsIntent::HeaderMenu {
                depth,
                anchor: Rect::new(at.x, at.y, 0.0, 0.0),
            },
            tx,
        );
    })
}

fn with_band<S: 'static>(sh: &Shared<S>, s: &S, bar: El, cx: HeaderCx) -> El {
    match sh.band.as_ref().and_then(|f| f(s, cx)) {
        Some(band) => col().w_full().child(bar).child(band),
        None => bar,
    }
}

fn reveal_target<S>(
    sh: &Shared<S>,
    title: &str,
    key: &str,
    (left_clipped, vis_w): (bool, f32),
) -> El {
    let label = sh.labels.reveal.replace("{name}", title);
    el().key(keys::reveal(key))
        .role("button")
        .label(label)
        .w(vis_w)
        .pin(
            (!left_clipped).then_some(0.0),
            Some(0.0),
            left_clipped.then_some(0.0),
            Some(0.0),
        )
}
