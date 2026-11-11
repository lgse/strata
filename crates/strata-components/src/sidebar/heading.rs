//! The PINNED and DEVICES headings. In the full sidebar: the title, a rule filling the width,
//! the count while collapsed, and a chevron at the right edge, which swaps between down and
//! right rather than rotating. In the rail: the chevron alone, centred, with the section's
//! name as its tooltip. Either way it is a button named after its section, with an expanded
//! state and the count as its description. A click toggles the section without taking focus;
//! from the keyboard it is a stop in the list, toggled by Enter or Space (and 10xer `l`).

use rune::{El, Key, KeyEvent, Tx, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::slots::sidebar as slot;

use super::intent::{Section, SidebarIntent};
use super::rows::{Ctx, tenxer_key};

/// The words a heading's description says: "3 items", "1 item".
pub fn count_words(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

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
    let c = &look.kit.c;
    let chevron_name = if collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    let chevron = icon(look.icon(chevron_name), look.icon_size.md)
        .key("chevron")
        .color(c.icon_faint)
        .a11y_hidden();
    let mut h = row()
        .key(format!("{}-heading-{}", ctx.key, section.id()))
        .role("button")
        .label(section.title())
        .description(count_words(rows))
        .expanded(!collapsed)
        .w_full()
        .h(slot::heading_height(look))
        .radius(look.kit.radius.md)
        .center_y()
        .focusable(true)
        // A click toggles without moving focus (it stays in the files); the keyboard still
        // stops here. `preserve_focus_on_press` keeps focus only off a focusable element.
        .keeps_text_selection()
        .hover(if ctx.state.dragging() {
            rune::Color::TRANSPARENT
        } else {
            slot::hover_fill(look)
        });
    if ctx.rail {
        h = h.center().child(chevron).help(section.title());
    } else {
        h = h
            .px(look.space.md)
            .gap(look.space.sm)
            .child(
                text(section.title())
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
        h = h.child(chevron);
    }
    let access = ctx.access;
    let tenxer = ctx.tenxer;
    let toggle = move |s: &mut S, tx: &mut Tx, emit: &super::Emit<S>| {
        super::rows::flush_reordered(s, tx, access, emit, None);
        let collapsed = access(s).toggle(section);
        emit(s, SidebarIntent::SectionToggled { section, collapsed }, tx);
    };
    let press_emit = ctx.emit.clone();
    let key_emit = ctx.emit.clone();
    h.on_press_tx(move |s: &mut S, tx: &mut Tx| toggle(s, tx, &press_emit))
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let activate = |s: &mut S, tx: &mut Tx| {
                toggle(s, tx, &key_emit);
                true
            };
            match &ev.key {
                Key::Enter => activate(s, tx),
                Key::Text(t) if t == " " => activate(s, tx),
                _ if tenxer => tenxer_key(s, ev, tx, &key_emit, activate),
                _ => false,
            }
        })
}
