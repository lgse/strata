//! The size badge on each row: "12 items", "No items" or a file's size, as the host words it
//! ([`ColumnSources::badge`](super::ColumnSources::badge)), and the row's accessible
//! description that carries the same text (Strata's `.file-size`, `src/style.css`, and
//! `src/ui/browser/columns/rows/`, `upstream/main` 35ee677ab).
//!
//! The badge is paint: an opaque chip pinned over the end of the row's name, as Strata's
//! overlay is. Its visibility is Rune's hover-group paint, so the pointer crossing rows commits
//! nothing: the row's content is a hover group whose hover fades in over 120 ms and goes at
//! once; the chip's `group_opacity` rests at 1 on a selected or open-path row, else at 0, and
//! rises to 1 under the hover. In keyboard mode (C1e's `row_hover` off) a transparent cover
//! over the row content takes the hover alone (`El::hover_alone`, no handlers, hidden from
//! assistive technology), so the group rests and a hovered row's chip hides at once; the
//! pointer move that ends keyboard mode removes the cover, and the group's hover, still the
//! same element, fades in afresh. Its rest changes land at once (an instant opacity motion).
//!
//! Rows are memoised by C1e on the look's colours, type, space and row heights, not on what
//! only the badge reads, so the content slot's version hashes that too: the `columns.badge.*`
//! tokens, the highlight fill and whether animations are on.

use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{Anim, Curve, Easing, El, el, row, text};
use strata_kit::look::Look;
use strata_kit::slots::columns as tokens;

use super::keys::BADGE;
use super::view::Shared;
use crate::collection::element::{ContentSlot, Describe, RowCx};

/// The hover's fade in: Strata's `transition: opacity 120ms ease-out`.
const FADE_IN: Anim = Anim::new(Curve::tween(0.12, Easing::EASE_OUT));

/// Column `serial`'s badge text for the row `rcx` builds, when the setting shows badges and the
/// row isn't being renamed.
fn shown_text<'s, S: 'static>(
    sh: &Shared<S>,
    s: &'s S,
    serial: u64,
    rcx: &RowCx<'_>,
) -> Option<Cow<'s, str>> {
    if rcx.state.renaming || !(sh.show_sizes)(s) {
        return None;
    }
    let p = (sh.parts)(s);
    let depth = p.state.depth_of(serial).filter(|d| *d < p.sources.len())?;
    p.sources.badge(depth, rcx.index).filter(|t| !t.is_empty())
}

/// The memo version of a row's badge: its text, and what of the look only the badge reads.
fn version_of(text: Option<&str>, look: &Look) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    text.hash(&mut h);
    look.animations.hash(&mut h);
    look.fill.highlight.hash(&mut h);
    for v in [
        tokens::badge_radius(look),
        tokens::badge_pad(look),
        tokens::badge_inset(look),
    ] {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

/// The row's content: C1e's icon and label, and the badge over the label's end when there is
/// one to show.
pub(super) fn content<S: 'static>(sh: &Rc<Shared<S>>, serial: u64) -> ContentSlot<S> {
    let (v, b) = (sh.clone(), sh.clone());
    ContentSlot::new(
        move |s: &S, rcx: &RowCx<'_>| {
            version_of(shown_text(&v, s, serial, rcx).as_deref(), rcx.look)
        },
        move |s: &S, rcx: &RowCx<'_>, icon: El, label: El| {
            let look = rcx.look;
            let content = row()
                .w_full()
                .h_full()
                .gap(look.space.sm)
                .center_y()
                .child(icon)
                .child(label);
            match shown_text(&b, s, serial, rcx) {
                Some(t) => {
                    // The hover lives on the hover group's track, which the row's own
                    // hover tint doesn't share: the fade is the badge's alone.
                    let live = (b.parts)(s)
                        .state
                        .depth_of(serial)
                        .and_then(|d| (b.parts)(s).state.column(d))
                        .is_none_or(|c| c.collection.row_hover());
                    let enter = if look.animations {
                        FADE_IN
                    } else {
                        Anim::INSTANT
                    };
                    // In keyboard mode a cover over the row takes the hover alone, so the
                    // group rests (the chip hides at once); the pointer move that ends keyboard
                    // mode removes it, and the group's hover fades in afresh.
                    let mut content = content
                        .hover_group()
                        .hover_motion(enter, Anim::INSTANT)
                        .child(chip(look, &t, rcx, live));
                    if !live {
                        content = content.child(
                            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                                .hover_alone()
                                .a11y_hidden(),
                        );
                    }
                    content
                }
                None => content,
            }
        },
    )
}

/// The chip: an opaque surface pill (the highlight on an open-path row) pinned to the row's
/// trailing edge, over the name.
fn chip(look: &Look, t: &str, rcx: &RowCx<'_>, hover_live: bool) -> El {
    let kit = &look.kit;
    let open_path = rcx.state.marks.active_path;
    let rest = if rcx.state.selected || open_path {
        1.0
    } else {
        0.0
    };
    // Keyboard mode: a hovered row shows only what it shows at rest (Strata's
    // `.keyboard-navigation row:hover:not(:selected) .file-size`). The cover already rests the
    // group then; this keeps the chip at rest should the cover not hold the hover.
    let hovered = if hover_live { 1.0 } else { rest };
    let fill = if open_path {
        look.fill.highlight
    } else {
        kit.c.surface_raised
    };
    let inset = tokens::badge_inset(look);
    // One leaf element, its line as tall as the chip so the text sits in its middle: a box
    // around a text costs a fifth more on each scroll frame (C2b's perf runs), and every row
    // has a chip.
    let h = (look.row.columns - 2.0 * inset).max(1.0);
    text(t.to_owned())
        .key(BADGE)
        .font(kit.ty.small.lh(h))
        .color(kit.c.text_muted)
        .pin(None, Some(inset), Some(0.0), None)
        .px(tokens::badge_pad(look))
        .radius(tokens::badge_radius(look))
        .fill(fill)
        .group_opacity(rest, hovered)
        .opacity_motion(Anim::INSTANT)
        // The row's description carries the text; the chip takes no input.
        .a11y_hidden()
        .inert()
}

/// The row's accessible description: the badge's text, under the same setting, and none while
/// the row is being renamed.
pub(super) fn describe<S: 'static>(sh: &Rc<Shared<S>>, serial: u64) -> Describe<S> {
    let (v, b) = (sh.clone(), sh.clone());
    Describe::new(
        move |s: &S, rcx: &RowCx<'_>| {
            let mut h = std::hash::DefaultHasher::new();
            shown_text(&v, s, serial, rcx).hash(&mut h);
            h.finish()
        },
        move |s: &S, rcx: &RowCx<'_>| shown_text(&b, s, serial, rcx).map(Cow::into_owned),
    )
}
