//! The crumb strip: ancestors as buttons, the current folder with its Copy path button, in a
//! sideways scroll viewport with edge fades and an overflow bar in its own row underneath.

use rune::{
    El, Key, KeyEvent, PointerButton, PointerEvent, Tx, WheelEvent, col, el, icon, row, text,
};
use strata_kit::icons::IconName;

use super::build::{Ctx, crumb_key, route};
use super::data::{CRUMB_CHARS, Crumb, elide_middle};
use super::drops;
use super::intent::{LocationIntent, Via};
use super::state::COPIED_FOR;
use crate::contract::dnd::DropHover;
use crate::header;

/// A crumb's drop ring and keyboard focus ring, in points: Strata's 1 px inset accent ring (a
/// hairline the Look has no token for, as C8's drop outline).
const RING: f32 = 1.0;

/// The strip's viewport key under the bar keyed `key`.
pub(crate) fn viewport_key(key: &str) -> String {
    format!("{key}-crumbs")
}

pub(super) fn view<S: 'static>(cx: &Ctx<'_, S>) -> El {
    let look = cx.look;
    let viewport = viewport_key(cx.key);
    let crumbs = &cx.trail.crumbs;
    let n = crumbs.len();
    let mut content = row()
        .key(format!("{}-trail-{}", cx.key, cx.state.trail_generation()))
        .role("list")
        .label(cx.labels.crumbs.clone())
        .h(cx.row())
        .items_center()
        .gap(look.space.xxs);
    for (i, crumb) in crumbs.iter().enumerate() {
        content = content.child(if i + 1 == n {
            current(cx, crumb, n)
        } else {
            ancestor(cx, i, crumb, n)
        });
        if i + 1 < n && !crumb.joins_next {
            content = content.child(
                text("/")
                    .key(format!("{}-sep-{i}", cx.key))
                    .font(look.kit.ty.ui)
                    .color(look.kit.c.text_weak)
                    .a11y_hidden(),
            );
        }
    }
    // The empty space after the last crumb: a press there opens the entry, as on a separator.
    content = content.child(
        el().key(format!("{}-crumbs-space", cx.key))
            .grow(1.0)
            .min_w(look.space.xl)
            .h(cx.row())
            .a11y_hidden(),
    );
    // Each navigation mounts a new row (keyed by its count), and so does the strip coming back
    // from the entry, so its first layout scrolls the strip to its end: the current crumb shows
    // after a navigation, a submit or a cancel.
    let access = cx.access;
    let reveal_key = viewport.clone();
    content = content.on_layout_tx(move |s: &mut S, _, tx: &mut Tx| {
        if access(s).take_reveal() {
            tx.scroll_to_x(reveal_key.clone(), f32::MAX);
        }
    });

    let (feed, bar) = (cx.state.feed.clone(), cx.state.bar.clone());
    let wheel_feed = cx.state.feed.clone();
    let wheel_key = viewport.clone();
    let text = cx.trail.text.clone();
    let press_emit = cx.emit.clone();
    let height = cx.height();
    let strip = row()
        .key(viewport.clone())
        // The row inside is the crumbs' list; the viewport only scrolls and takes presses.
        .role("none")
        .w_full()
        .h(cx.row())
        .scroll_x()
        .scroll_edge_fade(look.space.xl)
        .on_scroll_offset(move |p| {
            feed.set(p);
            bar.follow(p);
        })
        // Rune scrolls a sideways viewport only with sideways deltas; a mouse wheel turns it
        // here, as GTK's breadcrumbs do (Rune draft U-C6-5).
        .on_wheel(move |_: &mut S, ev: WheelEvent, tx: &mut Tx| {
            let p = wheel_feed.get();
            if ev.dy.abs() <= ev.dx.abs() || p.max[0] <= 0.0 {
                return false;
            }
            tx.scroll_to_x(
                wheel_key.clone(),
                (p.offset[0] - ev.dy).clamp(0.0, p.max[0]),
            );
            tx.live_only();
            true
        })
        // A press on a separator, the current crumb or the empty space opens the entry; the
        // crumb buttons take their own presses.
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            let polled = access(s).begin_edit(&text);
            route(&press_emit, s, polled, tx);
        })
        // The hierarchy menu opens under the whole bar at the press's x, so it never covers the
        // crumbs it lists.
        .on_secondary_press_local(move |s: &mut S, p: PointerEvent, _: &mut Tx| {
            access(s).open_menu(p.window_x, p.window_y - p.y + height);
        })
        .child(content);

    let bar_el =
        cx.state
            .bar
            .build::<S>(&format!("{}-crumbs-bar", cx.key), &viewport, cx.bar_look());
    // The bar lays itself along its viewport's bottom edge; hung from the viewport's origin
    // moved down by the bar's row, it sits in that row, under the labels. (A zero-size anchor,
    // not a frame over the crumbs: a frame there is what a system drag's hit test, P1's, would
    // find over them.)
    let bar_row = crate::scroll::anchor(0.0, cx.band(), bar_el);
    let column = col()
        .w_full()
        .h(cx.height())
        .min_w(0.0)
        .child(strip)
        .child(bar_row);
    cx.state.bar.area(column)
}

/// An ancestor: a button that opens its folder, a new tab or window on a middle click, and a
/// drop zone.
fn ancestor<S: 'static>(cx: &Ctx<'_, S>, i: usize, crumb: &Crumb, n: usize) -> El {
    let look = cx.look;
    let k = &look.kit;
    let hovered = matches!(cx.state.drop_hover(), Some((j, DropHover::Accepting)) if j == i);
    let mut button = row()
        .key(crumb_key(cx.key, i))
        .role("button")
        .label(crumb.label.clone())
        .description(crumb.description.clone())
        .focusable(true)
        .h(cx.row())
        .px(look.space.xs)
        .gap(look.space.xxs)
        .items_center()
        .radius(k.radius.sm)
        .hover(look.fill.highlight)
        .press_scale(0.97)
        .focus_ring(RING, 0.0, k.c.border_focus);
    if let Some(badge) = crumb.badge {
        button = button.child(
            icon(look.icon(badge), look.icon_size.sm)
                .color(k.c.icon_muted)
                .a11y_hidden(),
        );
    }
    // Strata's crumbs are dim until the pointer or a drag is over them.
    let lit = hovered || cx.state.hovered_crumb() == Some(i);
    button = button.child(
        text(elide_middle(&crumb.label, CRUMB_CHARS).into_owned())
            .font(k.ty.ui)
            .color(if lit { k.c.text_base } else { k.c.text_weak })
            .a11y_hidden(),
    );
    if hovered {
        // Strata's drop look: the accent tint and an accent ring inside the edge.
        button = button
            .fill(look.fill.highlight)
            .outline(RING, -RING, k.c.border_focus);
    }
    let id = crumb.id.clone();
    let (press_id, key_id) = (id.clone(), id.clone());
    let (emit, key_emit) = (cx.emit.clone(), cx.emit.clone());
    let open_in_new = cx.open_in_new;
    let access = cx.access;
    button = button
        .on_hover_change(move |s: &mut S, over: bool, _: &mut Tx| access(s).hover_crumb(i, over))
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            let id = press_id.clone();
            let intent = if tx.button() == PointerButton::Middle {
                let m = tx.mods();
                if m.ctrl || m.shift {
                    LocationIntent::OpenInNewWindow { id }
                } else {
                    LocationIntent::OpenInNewTab { id }
                }
            } else {
                LocationIntent::Open {
                    id,
                    via: Via::Click,
                }
            };
            emit(s, intent, tx);
        })
        // Return presses a focused crumb, as GTK's buttons take it (Rune gives a focused
        // button's Return to the default button).
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            if ev.key == Key::Enter && !(ev.mods.ctrl || ev.mods.alt || ev.mods.cmd) {
                let id = key_id.clone();
                key_emit(
                    s,
                    LocationIntent::Open {
                        id,
                        via: Via::Click,
                    },
                    tx,
                );
                return true;
            }
            false
        });
    if open_in_new {
        button = button.pointer_buttons([PointerButton::Middle]);
    }
    let button = cx
        .dnd
        .drop_zone_keyed(button, crumb_key(cx.key, i), drops::zone(cx, i, &id));
    el().key(format!("{}-item-{i}", cx.key))
        .role("listitem")
        .label(crumb.label.clone())
        .position_in_set(i + 1, n)
        .child(button)
}

/// The current folder: a label in the accent, and the Copy path button.
fn current<S: 'static>(cx: &Ctx<'_, S>, crumb: &Crumb, n: usize) -> El {
    let look = cx.look;
    let k = &look.kit;
    let copied = cx.state.copied();
    let label = if copied {
        cx.labels.copied.clone()
    } else {
        cx.labels.copy.clone()
    };
    let (access, emit) = (cx.access, cx.emit.clone());
    let icon_name = if copied {
        IconName::Check
    } else {
        IconName::Copy
    };
    // The header's action look; its tooltip names it.
    let copy = header::action(look, icon_name, &label)
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            access(s).show_copied(tx.clock());
            // A window at rest polls only at the wakes it asked for.
            tx.wake_at(tx.clock() + COPIED_FOR);
            emit(s, LocationIntent::CopyPath, tx);
        })
        .build(k)
        .key(format!("{}-copy", cx.key));
    let mut name = row()
        .key(format!("{}-current", cx.key))
        .role("none")
        .h(cx.row())
        .px(look.space.xs)
        .gap(look.space.xxs)
        .items_center();
    if let Some(badge) = crumb.badge {
        name = name.child(
            icon(look.icon(badge), look.icon_size.sm)
                .color(k.c.icon_accent)
                .a11y_hidden(),
        );
    }
    name = name.child(
        text(elide_middle(&crumb.label, CRUMB_CHARS).into_owned())
            .font(k.ty.ui_medium)
            .color(k.c.text_accent)
            .label(crumb.label.clone())
            .description(crumb.description.clone()),
    );
    row()
        .key(format!("{}-item-{}", cx.key, n - 1))
        .role("listitem")
        .label(crumb.label.clone())
        .position_in_set(n, n)
        .items_center()
        .child(name)
        .child(copy)
}
