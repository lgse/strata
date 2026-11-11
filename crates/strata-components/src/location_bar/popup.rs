//! The suggestion list: anchored under the entry at its width, taking no focus, on rune-kit's
//! public popover parts. Not rune-kit's `PopupList`, whose rows can't draw matched ranges
//! (Rune draft U-C6-4).

use std::hash::{DefaultHasher, Hash, Hasher};

use rune::{Color, El, GrowthSpec, ScrollSpec, Tx, col, icon, memo, row, text};
use rune_kit::{Placement, popover};
use strata_kit::look::Look;

use super::build::{Access, Ctx, Emit, field_key, option_key, route};
use super::data::Candidate;
use super::state::HandlerSlot;
use crate::contract::MatchedText;

/// The most rows the list shows before it scrolls (Strata's 280 px over 32 px rows).
const ROWS_SHOWN: f32 = 8.0;

pub(super) fn view<S: 'static>(cx: &Ctx<'_, S>, width: f32, layer: &mut Vec<El>) {
    let look = cx.look;
    let k = &look.kit;
    let shown = cx.state.suggestions();
    let n = shown.len();
    let active = cx.state.highlighted();
    let row_h = look.row.list;
    // Rune builds every row's element on each commit; only the mounted ones lay out. A row is
    // rebuilt only when its list, its highlight or the look changes, so an arrow step builds
    // two rows, not the whole list.
    let list = cx.state.suggestions_generation();
    // A row kept by its memo calls the handler this view hands the slot, not the one it was
    // built with.
    cx.state.handler.set(cx.emit.clone());
    let look_key = look_key(look);
    let mut rows = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtualize(row_h)
        .reveal_selected(true);
    for (i, c) in shown.iter().enumerate() {
        let is_active = active == Some(i);
        rows = rows.child(memo(
            option_key(cx.key, i),
            (list, i, n, is_active, look_key),
            || suggestion(look, c, i, n, is_active, cx.access, &cx.state.handler),
        ));
    }
    let viewport = col()
        .key(format!("{}-suggestions", cx.key))
        .role("listbox")
        .label(cx.labels.suggestions.clone())
        .w_full()
        .h((n as f32 * row_h).min(ROWS_SHOWN * row_h))
        .min_h(row_h.min(n as f32 * row_h))
        .shrink()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(look.space.lg)
        .scroll_margin(look.space.lg)
        .clip_soft(0.0)
        .child(rows);
    let content = col()
        .key(format!("{}-suggestions-panel", cx.key))
        .role("none")
        .w(width)
        .padding(look.space.xxs)
        // The entry keeps the keyboard while the pointer picks a row.
        .preserve_focus_on_press(true)
        .on_press(|_: &mut S| {})
        .instant_changes()
        .child(viewport);
    layer.push(popover::anchored(
        k,
        popover::raised(k, content, k.radius.md),
        &field_key(cx.key),
        Placement::BottomStart,
        look.space.xs,
    ));
}

/// One suggestion: its icon, its label with the matched ranges in the accent, its parent.
fn suggestion<S: 'static>(
    look: &Look,
    c: &Candidate,
    i: usize,
    n: usize,
    active: bool,
    access: Access<S>,
    handler: &HandlerSlot,
) -> El {
    let k = &look.kit;
    let handler = handler.clone();
    let label = MatchedText::new(c.label.clone(), c.matched.iter().cloned())
        .font(k.ty.ui_compact)
        .truncate(true)
        .build(k);
    row()
        .role("option")
        .label(c.label.clone())
        .description(c.hint.clone())
        .position_in_set(i + 1, n)
        .focus_selected(active)
        .w_full()
        .h(look.row.list)
        .px(look.space.sm)
        .gap(look.space.sm)
        .items_center()
        .radius(k.radius.sm)
        .fill(if active {
            k.c.hover
        } else {
            Color::TRANSPARENT
        })
        // A tint only: the pointer never moves the keyboard's highlight. Rune reports a hover
        // when a row scrolls under a pointer that isn't moving, so Down past the list's end
        // would hand the highlight to whatever row the pointer rests on (review round 1, B1).
        // Strata's list has no hover highlight either.
        .hover(k.c.hover)
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            let polled = access(s).choose(i);
            if let Some(emit) = handler.get::<Emit<S>>() {
                route(&emit, s, polled, tx);
            }
        })
        .child(
            icon(look.icon(c.icon), look.icon_size.md)
                .color(k.c.icon_muted)
                .a11y_hidden(),
        )
        .child(row().grow(1.0).min_w(0.0).a11y_hidden().child(label))
        .child(
            text(c.hint.clone())
                .font(k.ty.small)
                .color(k.c.text_weak)
                .truncate()
                .shrink()
                .a11y_hidden(),
        )
}

/// Everything in the look a suggestion row reads, as a memo version.
fn look_key(look: &Look) -> u64 {
    let mut h = DefaultHasher::new();
    look.kit.c.hash(&mut h);
    look.kit.ty.hash(&mut h);
    look.kit.radius.hash(&mut h);
    look.space.hash(&mut h);
    look.icon_size.hash(&mut h);
    look.row.hash(&mut h);
    look.icons.name.hash(&mut h);
    (look.icons.icon as usize).hash(&mut h);
    h.finish()
}
