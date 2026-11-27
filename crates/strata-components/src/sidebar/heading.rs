// SPDX-License-Identifier: MIT

use rune::{El, KeyEvent, Side, Tx, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::slots::sidebar as slot;

use super::intent::{Section, SidebarIntent};
use super::rows::{ACTION_SLOT, Ctx, action_icon_size, tenxer_key};

/// Whether a heading draws its count: collapsed, with at least one row.
pub(crate) fn shows_count(collapsed: bool, rows: usize) -> bool {
    collapsed && rows > 0
}

pub(crate) fn heading<S: 'static>(
    ctx: &Ctx<'_, S>,
    section: Section,
    rows: usize,
    collapsed: bool,
) -> El {
    let look = ctx.look;
    let h = row()
        .key(format!("{}-heading-{}", ctx.key, section.id()))
        .role("button")
        .label(ctx.labels.heading(section))
        .description((ctx.labels.rows)(rows))
        .expanded(!collapsed)
        .w_full()
        .h(slot::heading_height(look))
        .radius(look.kit.radius.md)
        .center_y()
        .focusable(true)
        .keeps_text_selection()
        .hover(if ctx.state.dragging() {
            rune::Color::TRANSPARENT
        } else {
            slot::hover_fill(look)
        });
    let h = heading_body(ctx, h, section, (rows, collapsed));
    let access = ctx.access;
    let toggle = move |s: &mut S, tx: &mut Tx, emit: &super::Emit<S>| {
        let collapsed = access(s).toggle(section);
        emit(s, SidebarIntent::SectionToggled { section, collapsed }, tx);
    };
    let press_emit = ctx.emit.clone();
    let mut h = h.on_press_tx(move |s: &mut S, tx: &mut Tx| toggle(s, tx, &press_emit));
    if ctx.tenxer {
        let key_emit = ctx.emit.clone();
        h = h.on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let activate = |s: &mut S, tx: &mut Tx| {
                toggle(s, tx, &key_emit);
                true
            };
            tenxer_key(s, ev, tx, &key_emit, activate)
        });
    }
    h
}

fn heading_body<S>(
    ctx: &Ctx<'_, S>,
    h: El,
    section: Section,
    (rows, collapsed): (usize, bool),
) -> El {
    let look = ctx.look;
    let c = &look.kit.c;
    let chevron_name = if collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    let chevron = icon(look.icon(chevron_name), action_icon_size(look))
        .key("chevron")
        .color(c.icon_accent)
        .opacity(0.65)
        .a11y_hidden();
    if ctx.rail {
        return h
            .center()
            .child(chevron)
            .help(ctx.labels.heading(section))
            .help_side(Side::Right);
    }
    let chevron_slot = row()
        .key("chevron-slot")
        .size(ACTION_SLOT, ACTION_SLOT)
        .center()
        .child(chevron)
        .a11y_hidden();
    let mut h = h
        .pl(slot::ROW_INSET)
        .pr(super::build::full_row_trailing_pad(look))
        .gap(look.space.sm)
        .child(
            text(ctx.labels.heading(section))
                .key("title")
                .font(look.ty.heading)
                .color(c.text_muted)
                .a11y_hidden(),
        )
        .child(
            el().key("rule")
                .h(1.0)
                .grow(1.0)
                .fill(slot::divider_color(look))
                .a11y_hidden(),
        );
    if shows_count(collapsed, rows) {
        h = h.child(
            text(rows.to_string())
                .key("count")
                .font(look.ty.heading)
                .color(c.text_faint)
                .a11y_hidden(),
        );
    }
    h.child(chevron_slot)
}
