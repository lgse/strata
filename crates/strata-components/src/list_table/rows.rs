//! The rows' cells (C1e's content slot), the middle-truncated name, and the hit regions C1b
//! reads.

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
///
/// Used by: C4, scripts.
pub fn cell_key(id: &str) -> String {
    format!("cell-{id}")
}

/// The name, cut in the middle so the extension stays visible (Strata's List Name column),
/// keyed [`LABEL`] as C1e's own label is.
pub(super) fn label<S: 'static>() -> Slot<S> {
    Slot::pure(|_: &S, rcx: &RowCx<'_>| {
        let kit = &rcx.look.kit;
        text(rcx.entry.name.to_owned())
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

/// The cells' slot: one gridcell per shown column, at the resolved widths.
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
                row()
                    .key(key)
                    .role("gridcell")
                    .label(rcx.entry.name)
                    .w(p.w)
                    .h_full()
                    .center_y()
                    .gap(fonts.gap)
                    .pr(pad)
                    .child(icon)
                    .child(label)
            }
            Some(f) => {
                let cell = f(s, rcx);
                match cell.content {
                    // The text is the cell: one element a cell keeps a row of five cheap to
                    // mount while scrolling.
                    CellContent::Text(t) => {
                        let color =
                            cell.color
                                .unwrap_or(if cell.muted { fonts.muted } else { fonts.base });
                        let font = if cell.mono { fonts.mono } else { fonts.text };
                        text(t)
                            .key(key)
                            .role("gridcell")
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
                            .role("gridcell")
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

pub(super) fn justify(e: El, a: Align) -> El {
    match a {
        Align::Start => e,
        Align::Center => e.justify_center(),
        Align::End => e.justify_end(),
    }
}

/// Which part of a row a point (in the row's own points) is on, as Strata's List counts it
/// (`browser/selection/pointer-intent`): the primary column's icon is content and its drawn
/// name is the name, the rest of that column and the row's padding are blank, and every other
/// column's cell is content, wherever the primary column sits.
pub(super) fn region(resolved: &Resolved, look: &Look, name: &str, local: Point) -> Region {
    let x = local.x - resolved.lead;
    let Some(p) = resolved.column_at(x) else {
        return Region::Gutter;
    };
    if !p.primary {
        return Region::Content;
    }
    let icon = look.icon_size.md + look.space.sm;
    let at = x - p.x;
    if at < icon {
        return Region::Content;
    }
    let pad = tokens::cell_pad(look);
    let (drawn, _) = rune::text::measure(name, &[], look.kit.ty.ui_compact, None);
    let room = (p.w - icon - pad).max(0.0);
    if at < icon + drawn.min(room) {
        Region::Name
    } else {
        Region::Gutter
    }
}
