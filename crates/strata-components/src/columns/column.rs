//! One column as the strip draws it: its shell (width, entrance and exit), its header, its
//! resize edge with the "Column width" caption, and the wash over it while a held file drag
//! would drop into it.

use std::rc::Rc;

use rune::{
    Anim, Cursor, Drag, El, Phase, PointerButton, Spring, SpringPair, Transition, Tx, col, el, row,
    text,
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

/// A column's entrance and exit, a sibling's fade, and a width easing to a spread or an
/// autofit: Strata's 220 ms `COLUMN_TRANSITION`.
const MOTION: Spring = Spring::new(0.22, 0.0);

/// How far a new column slides in from: Strata's `column-enter` keyframes.
const SLIDE: f32 = -24.0;

/// Column `depth`'s shell: its collection (with the header, bar and controllers), the line
/// along its edge, the resize edge over that line, the caption and the drop wash.
pub(super) fn shell<S: 'static>(sh: &Rc<Shared<S>>, s: &S, depth: usize) -> El {
    let p = (sh.parts)(s);
    let st = p.state;
    let look = p.look;
    let c = &st.columns[depth];
    let key = keys::column(&st.key, c.serial);
    let scale = width::scale(look);
    let w = st.drawn_width(depth, c, scale);
    let animated = look.animations;
    let pair = SpringPair::new(MOTION, MOTION);
    // The shell: a new column slides and fades in; a closed one narrows (its slot collapses)
    // and fades out; it takes no input while it leaves (Rune's presence). Without animations
    // both land at once.
    let shell_motion = if !animated {
        Transition::none()
    } else {
        let entrance = match c.entrance {
            Entrance::Slide => Transition::offset(SLIDE, 0.0).combined(Transition::opacity()),
            Entrance::Fade | Entrance::Still => Transition::none(),
        };
        Transition::asymmetric(entrance, Transition::opacity())
            .hold_layout()
            .springs(pair)
    };
    // The body: a sibling that takes a column's place fades in where the old one stood, and
    // the old one goes at once (`ColumnsState::swap_from`).
    let body_motion = if animated && c.entrance == Entrance::Fade {
        Transition::asymmetric(Transition::opacity(), Transition::none()).springs(pair)
    } else {
        Transition::none()
    };
    let list = collection(sh, s, c.serial).build(s);
    let body = col()
        .key(format!("{key}-body"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .transition(body_motion)
        .child(c.bar.area(list));
    let line = el()
        .pin(None, Some(0.0), Some(0.0), Some(0.0))
        .w(tokens::edge_width(look))
        .fill(tokens::fade(
            look,
            look.kit.c.std_border,
            tokens::edge_opacity(look),
        ))
        .a11y_hidden()
        .inert();
    let mut shell = col()
        .key(keys::slot(&st.key, c.slot))
        .w(w)
        .h_full()
        .min_h(0.0)
        .transition(shell_motion)
        .child(body)
        .child(line)
        .child(edge(sh, st, depth, c.serial, &key, scale, look));
    let resizing = st.resize.as_ref().is_some_and(|r| r.depth == depth);
    if st.edge_hover == Some(depth) || resizing {
        shell = shell.child(
            text(sh.labels.width.clone())
                .key(keys::caption(&key))
                .role("tooltip")
                .font(look.kit.ty.small)
                .color(look.kit.c.text_base)
                .px(look.space.sm)
                .py(look.space.xxs)
                .radius(look.round(look.kit.radius.sm))
                .fill(look.kit.c.surface_float)
                .border(tokens::caption_border(look), look.kit.c.border_muted)
                .pin(
                    None,
                    Some(tokens::header_height(look) + look.space.sm),
                    Some(EDGE_REACH + look.space.xs),
                    None,
                )
                .raise(),
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

/// The resize edge over the line on a column's right: it reaches [`EDGE_REACH`] either side,
/// over the next column too (raised, so it takes the press first), and resizes this column.
///
/// Under the pointer, and while it resizes, the line turns the accent at once (Strata's
/// `.column-resize-handle.resize-hover`): a line of its own over the column's, shown by the
/// edge's hover group, since Rune has no hover border colour (anomalyco/rune#328). A group's
/// own press doesn't hold its hover once the pointer leaves it, so a resize holds the line
/// lit by its rest. The edge lights C9's divider line at once too, while the "Column width"
/// caption waits [`CAPTION_DELAY`] on a child of the same size.
fn edge<S: 'static>(
    sh: &Rc<Shared<S>>,
    st: &ColumnsState,
    depth: usize,
    serial: u64,
    key: &str,
    scale: f32,
    look: &Look,
) -> El {
    let (drag, lit, hover) = (sh.clone(), sh.clone(), sh.clone());
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
    el().key(keys::edge(key))
        .pin(None, Some(0.0), Some(-EDGE_REACH), Some(0.0))
        .w(2.0 * EDGE_REACH)
        .cursor(Cursor::ResizeColumn)
        .a11y_hidden()
        .raise()
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
            // Only the last column's edge lights something the view doesn't paint itself
            // (the drawer's line, which the host reads in its poll).
            if depth + 1 != st.columns.len() {
                tx.live_only();
            }
        })
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            resize(&drag, s, serial, d, scale, tx);
        })
        .child(caption_hover)
        .child(
            el().key(keys::edge_accent(key))
                .pin(Some(EDGE_REACH - line), Some(0.0), None, Some(0.0))
                .w(line)
                .fill(look.kit.c.bg_accent)
                .group_opacity(rest, 1.0)
                .opacity_motion(Anim::INSTANT)
                .a11y_hidden()
                .inert(),
        )
}

/// A drag of a column's edge: only this column follows the pointer, between one standard
/// column and the area's cap; the release (a click too) gives every other open column the
/// same width, easing once, and hands the host the width; a cancelled drag puts the width
/// back. A second press on the same edge within [`AUTOFIT_WITHIN`] of the last fits the column to
/// its content instead (Strata's two presses, timed by the host's clock), even one that would
/// start a fresh drag: as Strata's `repeats_press`, any press counts, so a drag begun within
/// 400 ms of a click or another drag on that edge fits the column and resizes nothing.
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
                // The width lands with the pointer: an eased width would put rows where the
                // pointer isn't.
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

/// Fits column `depth` to the widest of its title and the rows on screen, within one standard
/// column and the area, easing there; the host saves the width (Strata saves autofit's capped
/// width). Only this column changes.
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

/// Column `depth`'s header: its title (middle-ellipsized), a spinner while it loads, and in the
/// targeted column the host's actions and, after the first column, the close button. The
/// active column's header is marked (the paste destination). A secondary press on the title or
/// blank header space opens the folder's menu; the actions keep their own. Its blank space
/// takes a click as focusing the column (C1e's `header_defers_press`), and a drag from it
/// bands.
pub(super) fn header<S: 'static>(sh: &Rc<Shared<S>>, s: &S, depth: usize, serial: u64) -> El {
    let p = (sh.parts)(s);
    let st = p.state;
    let look = p.look;
    let k = &look.kit;
    let key = keys::column(&st.key, serial);
    let active = depth == st.active;
    let targeted = depth == st.targeted();
    let loading = depth < p.sources.len() && p.sources.loading(depth);
    let title = if depth < p.sources.len() {
        p.sources.title(depth).to_owned()
    } else {
        String::new()
    };
    let color = if active {
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
        .child(
            text(title)
                .key(keys::title(&key))
                // The list is named by it already.
                .a11y_hidden()
                .font(tokens::header_title_font(look))
                .color(color)
                .truncate_middle()
                .shrink()
                .grow(1.0)
                .min_w(0.0),
        );
    if active {
        bar = bar.fill(tokens::header_fill(look)).child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), None)
                .h(tokens::header_indicator(look))
                .fill(k.c.bg_accent)
                .a11y_hidden()
                .inert(),
        );
    }
    if loading {
        bar = bar.child(
            el().key(keys::spinner(&key))
                .role("progressbar")
                .label(sh.labels.loading.clone())
                .child(
                    Spinner::new()
                        .size(look.icon_size.md)
                        .build(k)
                        .a11y_hidden(),
                ),
        );
    }
    let mut controls = row().gap(look.space.xxs).items_center();
    let mut any = false;
    if targeted {
        let cx = HeaderCx {
            depth,
            active,
            targeted,
            loading,
        };
        if let Some(actions) = sh.actions.as_ref().and_then(|f| f(s, cx)) {
            controls = controls.child(actions);
            any = true;
        }
        if depth > 0 {
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
                    .build(k)
                    // Not a focus stop: a press on it focuses the strip, which keeps the
                    // keyboard when the button goes with its column.
                    .focusable(false)
                    .key(keys::close(&key)),
            );
            any = true;
        }
    }
    if any {
        // A secondary press on a control is the control's: no folder menu.
        bar = bar.child(controls.on_secondary_press(|_: &mut S, _, tx: &mut Tx| tx.live_only()));
    }
    let menu = sh.clone();
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
