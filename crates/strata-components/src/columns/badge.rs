// SPDX-License-Identifier: MIT

use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{Anim, Curve, Easing, El, col, el, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::columns as tokens;

use super::keys::{BADGE, DETAIL};
use super::view::Shared;
use crate::collection::element::{ContentSlot, Describe, LABEL, RowCx};

/// The hover's fade in: Strata's `transition: opacity 120ms ease-out`.
const FADE_IN: Anim = Anim::new(Curve::tween(0.12, Easing::EASE_OUT));

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

fn detail_text<'s, S: 'static>(
    sh: &Shared<S>,
    s: &'s S,
    serial: u64,
    rcx: &RowCx<'_>,
) -> Option<Cow<'s, str>> {
    let p = (sh.parts)(s);
    let depth = p.state.depth_of(serial).filter(|d| *d < p.sources.len())?;
    if !p.sources.has_details(depth) {
        return None;
    }
    p.sources.detail(depth, rcx.index).filter(|t| !t.is_empty())
}

/// The line height of a detail line's type.
fn detail_line(look: &Look) -> f32 {
    let t = look.kit.ty.small;
    match t.line_height > 0.0 {
        true => t.line_height,
        false => t.size * 1.25,
    }
}

pub(super) fn detailed_height(look: &Look) -> f32 {
    (look.row.columns + 2.0 * detail_line(look) + look.space.xxs).round()
}

/// The name, and the detail under it when there is one: muted, wrapped to two lines.
fn with_detail(label: El, detail: Option<&str>, look: &Look) -> El {
    let Some(detail) = detail else {
        return label;
    };
    let kit = &look.kit;
    let line = text(detail.to_owned())
        .key(LABEL)
        .a11y_hidden()
        .font(kit.ty.small)
        .color(kit.c.text_muted)
        .wrap_anywhere()
        .max_lines(2)
        .w_full()
        .min_w(0.0);
    col()
        .shrink()
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space.xxs)
        .child(label)
        .child(col().key(DETAIL).w_full().min_w(0.0).child(line))
}

fn shows_tooltip(rcx: &RowCx<'_>) -> bool {
    !rcx.state.renaming
        && !rcx.state.marks.dragging
        && !rcx.state.marks.peek_anchor
        && !rcx.entry.name.is_empty()
}

/// Version of column `serial`'s hover tooltip for `rcx`.
fn tooltip_version<S: 'static>(sh: &Shared<S>, s: &S, serial: u64, rcx: &RowCx<'_>) -> u64 {
    if !shows_tooltip(rcx) {
        return 0;
    }
    let p = (sh.parts)(s);
    let Some(depth) = p.state.depth_of(serial).filter(|d| *d < p.sources.len()) else {
        return 0;
    };
    p.sources.tooltip_version(depth, rcx.index)
}

/// Column `serial`'s unconditional hover tooltip text for `rcx` (for example `"{name}\n{date}"`).
fn row_tooltip<'s, S: 'static>(
    sh: &Shared<S>,
    s: &'s S,
    serial: u64,
    rcx: &RowCx<'_>,
) -> Option<Cow<'s, str>> {
    if !shows_tooltip(rcx) {
        return None;
    }
    let p = (sh.parts)(s);
    let depth = p.state.depth_of(serial).filter(|d| *d < p.sources.len())?;
    p.sources
        .tooltip(depth, rcx.index)
        .filter(|t| !t.is_empty())
}

pub(super) fn content<S: 'static>(sh: &Rc<Shared<S>>, serial: u64) -> ContentSlot<S> {
    let (v, b) = (sh.clone(), sh.clone());
    ContentSlot::new(
        move |s: &S, rcx: &RowCx<'_>| {
            let mut h = std::hash::DefaultHasher::new();
            detail_text(&v, s, serial, rcx).hash(&mut h);
            tooltip_version(&v, s, serial, rcx).hash(&mut h);
            h.finish() ^ version_of(shown_text(&v, s, serial, rcx).as_deref(), rcx.look)
        },
        move |s: &S, rcx: &RowCx<'_>, row_icon: El, label: El| {
            let look = rcx.look;
            let tooltip = row_tooltip(&b, s, serial, rcx);
            let label = if tooltip.is_none() && shows_tooltip(rcx) {
                label.help_when_truncated(rcx.entry.name.to_owned())
            } else {
                label
            };
            let label = with_detail(label, detail_text(&b, s, serial, rcx).as_deref(), look);
            let mut middle = row().grow(1.0).h_full().min_w(0.0).center_y().child(label);
            let shown = shown_text(&b, s, serial, rcx);
            let live = shown.is_some()
                && (b.parts)(s)
                    .state
                    .depth_of(serial)
                    .and_then(|d| (b.parts)(s).state.column(d))
                    .is_none_or(|c| c.collection.row_hover());
            if let Some(t) = &shown {
                middle = middle.child(chip(look, t, rcx, live));
            }
            let mut content = row()
                .w_full()
                .h_full()
                .gap(look.space.sm)
                .center_y()
                .child(row_icon)
                .child(middle);
            if rcx.entry.kind.is_folder_like() {
                content = content.child(folder_chevron(look, rcx));
            }
            if let Some(tip) = tooltip {
                content = content.help(tip.into_owned());
            }
            if shown.is_some() {
                let enter = if look.animations {
                    FADE_IN
                } else {
                    Anim::INSTANT
                };
                content = content.hover_group().hover_motion(enter, Anim::INSTANT);
                if !live {
                    content = content.child(
                        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                            .hover_alone()
                            .a11y_hidden(),
                    );
                }
            }
            content
        },
    )
}

/// Trailing disclosure chevron on folder rows (Strata's `.file-chevron`).
fn folder_chevron(look: &Look, rcx: &RowCx<'_>) -> El {
    let opacity = if rcx.state.selected || rcx.state.marks.active_path {
        1.0
    } else {
        0.55
    };
    let chevron = icon(look.icon(IconName::ChevronRight), look.icon_size.sm)
        .color(look.kit.c.icon_accent)
        .a11y_hidden();
    if opacity < 1.0 {
        chevron.opacity(opacity)
    } else {
        chevron
    }
}

fn chip(look: &Look, t: &str, rcx: &RowCx<'_>, hover_live: bool) -> El {
    let kit = &look.kit;
    let open_path = rcx.state.marks.active_path;
    let rest = if rcx.state.selected || open_path {
        1.0
    } else {
        0.0
    };
    let hovered = if hover_live { 1.0 } else { rest };
    let fill = if open_path {
        look.fill.highlight
    } else {
        kit.c.surface_raised
    };
    let inset = tokens::badge_inset(look);
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

pub(super) fn describe<S: 'static>(sh: &Rc<Shared<S>>, serial: u64) -> Describe<S> {
    let (v, b) = (sh.clone(), sh.clone());
    Describe::new(
        move |s: &S, rcx: &RowCx<'_>| {
            let mut h = std::hash::DefaultHasher::new();
            described(&v, s, serial, rcx).hash(&mut h);
            shown_text(&v, s, serial, rcx).hash(&mut h);
            detail_text(&v, s, serial, rcx).hash(&mut h);
            if let Some((ver, _)) = &v.extra_description {
                ver(s, rcx.entry.id).hash(&mut h);
            }
            h.finish()
        },
        move |s: &S, rcx: &RowCx<'_>| {
            let mut parts: Vec<String> = Vec::new();
            if let Some(base) = described(&b, s, serial, rcx).filter(|t| !t.is_empty()) {
                parts.push(base.into_owned());
            }
            if let Some(badge) = shown_text(&b, s, serial, rcx).filter(|t| !t.is_empty())
                && parts.last().map(String::as_str) != Some(badge.as_ref())
            {
                parts.push(badge.into_owned());
            }
            if let Some(detail) = detail_text(&b, s, serial, rcx).filter(|t| !t.is_empty()) {
                parts.push(detail.into_owned());
            }
            if let Some(extra) = b
                .extra_description
                .as_ref()
                .and_then(|(_, text)| text(s, rcx.entry.id))
                .filter(|t| !t.is_empty())
            {
                parts.push(extra);
            }
            (!parts.is_empty()).then(|| parts.join(", "))
        },
    )
}

fn described<'s, S: 'static>(
    sh: &Shared<S>,
    s: &'s S,
    serial: u64,
    rcx: &RowCx<'_>,
) -> Option<Cow<'s, str>> {
    let p = (sh.parts)(s);
    p.state
        .depth_of(serial)
        .filter(|d| *d < p.sources.len())
        .and_then(|depth| p.sources.description(depth, rcx.index))
}
