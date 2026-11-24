// SPDX-License-Identifier: MIT

use rune::{
    Anim, Cursor, Curve, Easing, El, PointerEvent, Tx, WheelEvent, col, el, icon, row, text,
};
use strata_kit::icons::IconName;

use super::build::{Ctx, crumb_key, route};
use super::data::{CRUMB_CHARS, Crumb, elide_middle};
use super::drops;
use super::intent::{LocationIntent, Via};
use super::state::COPIED_FOR;
use crate::contract::dnd::DropHover;

const RING: f32 = 1.0;

/// The Copy path button's reveal and hide fade: Strata's `transition: opacity 160ms ease-out`.
const COPY_FADE: Anim = Anim::new(Curve::tween(0.16, Easing::EASE_OUT));

/// The Copy path button's hover fill alpha: Strata's `background: alpha(@strata_accent, 0.15)`.
const COPY_HOVER_ALPHA: f32 = 0.15;

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
        .role("group")
        .label(cx.labels.crumbs.clone())
        .h(cx.row())
        .items_center();
    for (i, crumb) in crumbs.iter().enumerate() {
        content = content.child(if i + 1 == n {
            current(cx, crumb, n)
        } else {
            ancestor(cx, i, crumb)
        });
        if i + 1 < n && !crumb.joins_next {
            content = content.child(gap(cx, format!("sep-gap-{i}"))).child(
                row()
                    .key(format!("{}-sep-{i}", cx.key))
                    .h(cx.row())
                    .items_center()
                    .a11y_hidden()
                    .on_press_tx(opens_the_entry(cx))
                    .child(text("/").font(look.kit.ty.ui).color(look.kit.c.text_weak)),
            );
        }
        if i + 1 < n {
            content = content.child(gap(cx, format!("gap-{i}")));
        }
    }
    // The empty space after the last crumb: a press there opens the entry, as on a separator.
    content = content.child(
        el().key(format!("{}-crumbs-space", cx.key))
            .grow(1.0)
            .min_w(look.space.xl)
            .h(cx.row())
            .a11y_hidden()
            .on_press_tx(opens_the_entry(cx)),
    );
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
    let menu_bottom = cx.menu_bottom();
    let strip = row()
        .key(viewport.clone())
        .cursor(Cursor::Text)
        .w_full()
        .h(cx.row())
        .scroll_x()
        .scroll_edge_fade(look.space.xl)
        .on_scroll_offset(move |p| {
            feed.set(p);
            bar.follow(p);
        })
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
        .on_secondary_press_local(move |s: &mut S, p: PointerEvent, _: &mut Tx| {
            access(s).open_menu(p.window_x, p.window_y - p.y + menu_bottom);
        })
        .child(content);

    let bar_el =
        cx.state
            .bar
            .build::<S>(&format!("{}-crumbs-bar", cx.key), &viewport, cx.bar_look());
    let bar_row = crate::scroll::anchor(0.0, cx.band(), bar_el);
    let column = col()
        .w_full()
        .h(cx.row())
        .min_w(0.0)
        .child(strip)
        .child(bar_row);
    cx.state.bar.area(column)
}

fn gap<S: 'static>(cx: &Ctx<'_, S>, name: String) -> El {
    el().key(format!("{}-{name}", cx.key))
        .w(cx.look.space.xxs)
        .h(cx.row())
        .a11y_hidden()
        .on_press_tx(opens_the_entry(cx))
}

fn opens_the_entry<S: 'static>(cx: &Ctx<'_, S>) -> impl Fn(&mut S, &mut Tx) + 'static {
    let (access, emit, text) = (cx.access, cx.emit.clone(), cx.trail.text.clone());
    move |s: &mut S, tx: &mut Tx| {
        let polled = access(s).begin_edit(&text);
        route(&emit, s, polled, tx);
    }
}

fn ancestor<S: 'static>(cx: &Ctx<'_, S>, i: usize, crumb: &Crumb) -> El {
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
        .cursor(Cursor::Pointer)
        .hover(look.fill.highlight)
        .press_scale(0.97)
        .focus_ring(RING, 0.0, k.c.border_focus);
    if let Some(badge) = crumb.badge {
        button = button.child(
            icon(look.icon(badge), look.icon_size.sm)
                .color(k.c.icon_accent)
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
    ancestor_handlers(cx, button, i, &crumb.id)
}

fn ancestor_handlers<S: 'static>(cx: &Ctx<'_, S>, button: El, i: usize, id: &str) -> El {
    let press_id = id.to_owned();
    let emit = cx.emit.clone();
    let access = cx.access;
    let mut button = button
        .press_on_return()
        .on_hover_change(move |s: &mut S, over: bool, _: &mut Tx| access(s).hover_crumb(i, over))
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            emit(
                s,
                LocationIntent::Open {
                    id: press_id.clone(),
                    via: Via::Click,
                },
                tx,
            );
        });
    if cx.open_in_new {
        let middle_id = id.to_owned();
        let middle_emit = cx.emit.clone();
        button = button.on_middle_press(move |s: &mut S, tx: &mut Tx| {
            let m = tx.mods();
            let id = middle_id.clone();
            let intent = if m.ctrl || m.shift {
                LocationIntent::OpenInNewWindow { id }
            } else {
                LocationIntent::OpenInNewTab { id }
            };
            middle_emit(s, intent, tx);
        });
    }
    cx.dnd
        .drop_zone_keyed(button, crumb_key(cx.key, i), drops::zone(cx, i, id))
}

fn copy_path_button<S: 'static>(cx: &Ctx<'_, S>, fade: Anim) -> El {
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
    let rest = if copied { 1.0 } else { 0.0 };
    row()
        .key(format!("{}-copy", cx.key))
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .size(cx.row(), cx.row())
        .center()
        .radius(k.radius.sm)
        .cursor(Cursor::Pointer)
        .hover(k.alpha(k.c.bg_accent, COPY_HOVER_ALPHA))
        .hover_motion(fade, fade)
        .press(k.c.pressed)
        .focus_ring(RING, 0.0, k.c.border_focus)
        .group_opacity(rest, 1.0)
        .opacity_motion(fade)
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            access(s).show_copied(tx.clock());
            // A window at rest polls only at the wakes it asked for.
            tx.wake_at(tx.clock() + COPIED_FOR);
            emit(s, LocationIntent::CopyPath, tx);
        })
        .press_on_return()
        .child(
            icon(look.icon(icon_name), look.icon_size.md)
                .key("icon")
                .color(k.c.icon_accent)
                .a11y_hidden(),
        )
}

/// The current folder: a label in the accent, and the Copy path button.
fn current<S: 'static>(cx: &Ctx<'_, S>, crumb: &Crumb, n: usize) -> El {
    let look = cx.look;
    let k = &look.kit;
    let fade = if look.animations {
        COPY_FADE
    } else {
        Anim::INSTANT
    };
    let copy = copy_path_button(cx, fade);
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
        .role("none")
        .h(cx.row())
        .items_center()
        .hover_group()
        .hover_motion(fade, fade)
        .on_press_tx(opens_the_entry(cx))
        .child(name)
        .child(copy)
}
