// SPDX-License-Identifier: MIT

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{Align, Color, El, TextStyle, row, text};
use strata_kit::look::Look;
use strata_kit::slots::list as tokens;

use super::Shared;
use super::column::CellContent;
use super::resolve::Resolved;
use crate::collection::element::{ContentSlot, LABEL, Region, RowCx, Slot};
use crate::contract::Point;

/// The key of a row's cell in column `id`, under its row (`row-{entry}/cell-{id}`).
pub fn cell_key(id: &str) -> String {
    format!("cell-{id}")
}

pub(super) fn label<S: 'static>() -> Slot<S> {
    Slot::pure(|_: &S, rcx: &RowCx<'_>| {
        let kit = &rcx.look.kit;
        let spans = rcx.state.marks.name_matches.spans(rcx.look);
        text(rcx.entry.name.to_owned())
            .spans(spans)
            .key(LABEL)
            .a11y_hidden()
            .font(kit.ty.ui_compact)
            .color(kit.c.text_base)
            .truncate_middle()
            .shrink()
            .min_w(0.0)
    })
}

/// What every row's cells draw with, worked out once per build rather than per row.
pub(super) struct Fonts {
    text: TextStyle,
    mono: TextStyle,
    base: Color,
    muted: Color,
    pad: f32,
    gap: f32,
}

impl Fonts {
    pub(super) fn of(look: &Look) -> Self {
        let kit = &look.kit;
        Self {
            text: kit.ty.ui_compact,
            mono: rune_kit::TableStyle::from_kit(kit).mono,
            base: kit.c.text_base,
            muted: kit.c.text_muted,
            pad: tokens::cell_pad(look),
            gap: look.space.sm,
        }
    }
}

pub(super) fn content<S: 'static>(
    shared: &Rc<Shared<S>>,
    resolved: &Rc<Resolved>,
    fonts: Fonts,
) -> ContentSlot<S> {
    let (version_shared, build_shared) = (shared.clone(), shared.clone());
    let (version_resolved, build_resolved) = (resolved.clone(), resolved.clone());
    let pad_bits = fonts.pad.to_bits();
    ContentSlot::new(
        move |s: &S, rcx: &RowCx<'_>| {
            let mut h = std::hash::DefaultHasher::new();
            version_resolved.fingerprint.hash(&mut h);
            pad_bits.hash(&mut h);
            rune::text::font_epoch().hash(&mut h);
            if let Some(f) = &version_shared.cells_version {
                f(s, rcx).hash(&mut h);
            }
            h.finish()
        },
        move |s: &S, rcx: &RowCx<'_>, icon: El, label: El| {
            cells(&build_shared, &build_resolved, &fonts, s, rcx, icon, label)
        },
    )
}

impl Fonts {
    fn primary_tooltip<S>(
        &self,
        shared: &Shared<S>,
        width: f32,
        (s, rcx): (&S, &RowCx<'_>),
    ) -> Option<String> {
        let provide = shared.tooltip.as_ref()?;
        if rcx.state.renaming
            || rcx.state.marks.dragging
            || rcx.state.marks.peek_anchor
            || rcx.entry.name.is_empty()
        {
            return None;
        }
        let room = (width - rcx.look.icon_size.md - self.gap - self.pad).max(0.0);
        let truncated = || rune::text::advance(rcx.entry.name, self.text) > room + 0.5;
        provide(s, rcx, &truncated).filter(|tip| !tip.is_empty())
    }
}

fn cells<S: 'static>(
    shared: &Shared<S>,
    resolved: &Resolved,
    fonts: &Fonts,
    s: &S,
    rcx: &RowCx<'_>,
    icon: El,
    label: El,
) -> El {
    let pad = fonts.pad;
    // The cells' widths add up to this row's, so none shrinks.
    let mut out = row().w(resolved.content).h_full().center_y();
    let mut parts = Some((icon, label));
    for p in &resolved.cols {
        let column = &shared.columns[p.def];
        let key = cell_key(column.id.as_str());
        let cell = match &column.cell {
            None => {
                let (icon, label) = parts.take().unwrap_or_else(|| (rune::el(), rune::el()));
                // No role and so no node of its own: the item is named by the entry already.
                let mut cell = row()
                    .key(key)
                    .role("none")
                    .label(rcx.entry.name)
                    .w(p.w)
                    .h_full()
                    .center_y()
                    .gap(fonts.gap)
                    .pr(pad)
                    .child(icon)
                    .child(label);
                if let Some(tip) = fonts.primary_tooltip(shared, p.w, (s, rcx)) {
                    cell = cell.help(tip);
                }
                cell
            }
            Some(f) => {
                let cell = f(s, rcx);
                match cell.content {
                    CellContent::Text(t) => {
                        let color =
                            cell.color
                                .unwrap_or(if cell.muted { fonts.muted } else { fonts.base });
                        let font = if cell.mono { fonts.mono } else { fonts.text };
                        text(t)
                            .key(key)
                            .w(p.w)
                            .px(pad)
                            .font(font)
                            .color(color)
                            .align(column.align)
                            .truncate()
                    }
                    // An element is cut at the cell's edge.
                    CellContent::El(e) => justify(
                        row()
                            .key(key)
                            .w(p.w)
                            .h_full()
                            .center_y()
                            .px(pad)
                            .clip_soft(0.0),
                        column.align,
                    )
                    .child(*e),
                }
            }
        };
        out = out.child(cell);
    }
    out
}

fn justify(e: El, a: Align) -> El {
    match a {
        Align::Start => e,
        Align::Center => e.justify_center(),
        Align::End => e.justify_end(),
    }
}

pub(super) fn region(resolved: &Resolved, look: &Look, name: &str, local: Point) -> Region {
    let x = local.x - resolved.lead;
    let Some(p) = resolved.column_at(x) else {
        return Region::Gutter;
    };
    if !p.primary {
        return Region::Gutter;
    }
    let inner_h = (tokens::row_height(look) - 2.0).max(0.0);
    let icon = look.icon_size.md + look.space.sm;
    let at = x - p.x;
    if at < icon {
        return if in_centered_band(local.y, inner_h, look.icon_size.md) {
            Region::Content
        } else {
            Region::Gutter
        };
    }
    let pad = tokens::cell_pad(look);
    let (drawn, text_h) = rune::text::measure(name, &[], look.kit.ty.ui_compact, None);
    let room = (p.w - icon - pad).max(0.0);
    let label_h = text_h.max(look.kit.ty.ui_compact.line_height);
    if at < icon + drawn.min(room) && in_centered_band(local.y, inner_h, label_h) {
        Region::Name
    } else {
        Region::Gutter
    }
}

fn in_centered_band(y: f32, container: f32, band: f32) -> bool {
    let margin = ((container - band) * 0.5).max(0.0);
    y >= margin && y < container - margin
}
