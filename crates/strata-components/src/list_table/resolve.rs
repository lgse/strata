//! Where each shown column sits at a viewport width: the one sizing rule the headings, the
//! cells and the hit regions all read, so what is drawn and what a press hits agree.

use std::hash::{Hash, Hasher};

use super::column::{Column, ColumnId};
use super::layout::{ColumnLayout, ColumnWidths, DropSite};

/// One shown column: its id, its index among the table's columns, and its left edge and width
/// in points, from the left of the first column.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Placed {
    pub id: ColumnId,
    pub def: usize,
    /// The primary column (the icon and the name).
    pub primary: bool,
    pub x: f32,
    pub w: f32,
}

/// The shown columns at one viewport width.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Resolved {
    pub cols: Vec<Placed>,
    /// The space before the first column and after the last: the row's inset and its inner
    /// padding (C1e's row frame).
    pub lead: f32,
    /// The columns' total width.
    pub content: f32,
    /// The text-size factor widths were scaled by.
    pub scale: f32,
    pub fingerprint: u64,
}

/// What the widths read: the person's widths, and the width being dragged right now.
pub(super) struct Inputs<'a> {
    pub layout: &'a ColumnLayout,
    pub widths: &'a ColumnWidths,
    /// A resize in progress: this column at this base width.
    pub live: Option<(&'a ColumnId, f32)>,
    /// The viewport's width, 0 before the first layout.
    pub viewport: f32,
    pub lead: f32,
    pub scale: f32,
}

/// The rule, after Strata's `ListColumnLayout` (`src/ui/browser_modes/list_columns.rs`): each
/// column takes its hand-set width (or the one being dragged), else its default, clamped to
/// its bounds, in base points times `scale`. The first fill column with no hand-set width
/// takes what the others leave of the viewport, and at least its own width; below that the
/// columns overflow and the table scrolls sideways.
pub(super) fn resolve<S>(defs: &[Column<S>], i: &Inputs<'_>) -> Resolved {
    let scale = if i.scale.is_finite() && i.scale > 0.0 {
        i.scale
    } else {
        1.0
    };
    let mut cols: Vec<Placed> = Vec::new();
    let mut filler = None;
    for id in i.layout.visible() {
        let Some(def) = defs.iter().position(|c| c.id == *id) else {
            continue;
        };
        let c = &defs[def];
        let set = match i.live {
            Some((live, w)) if live == id => Some(w),
            _ => i.widths.get(id),
        };
        if set.is_none() && c.fill && filler.is_none() {
            filler = Some(cols.len());
        }
        let w = c.bounded(set.unwrap_or(c.width)) * scale;
        cols.push(Placed {
            id: id.clone(),
            def,
            primary: c.is_primary(),
            x: 0.0,
            w,
        });
    }
    if let Some(f) = filler {
        let others: f32 = cols
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != f)
            .map(|(_, p)| p.w)
            .sum();
        let free = i.viewport - 2.0 * i.lead - others;
        let c = &defs[cols[f].def];
        let most = if c.max.is_finite() {
            c.max.max(c.min) * scale
        } else {
            f32::INFINITY
        };
        cols[f].w = cols[f].w.max(free.min(most));
    }
    let mut x = 0.0;
    for p in &mut cols {
        // Whole points, so neighbouring cells meet without a seam.
        p.w = p.w.round();
        p.x = x;
        x += p.w;
    }
    let mut h = std::hash::DefaultHasher::new();
    for p in &cols {
        p.id.hash(&mut h);
        p.w.to_bits().hash(&mut h);
    }
    i.lead.to_bits().hash(&mut h);
    scale.to_bits().hash(&mut h);
    Resolved {
        cols,
        lead: i.lead,
        content: x,
        scale,
        fingerprint: h.finish(),
    }
}

impl Resolved {
    /// The shown column at `x`, in points from the left of the first column.
    pub fn column_at(&self, x: f32) -> Option<&Placed> {
        self.cols.iter().find(|p| x >= p.x && x < p.x + p.w)
    }

    /// Where `dragged` drops with the pointer at `x` (from the left of the first column), as
    /// Strata's `column_drop_site` decides: before the first other shown column whose middle
    /// is right of the pointer, else after the last; `None` where it already is.
    pub fn drop_site(&self, dragged: &ColumnId, x: f32) -> Option<DropSite> {
        let original = self.cols.iter().position(|p| p.id == *dragged)?;
        let others: Vec<&Placed> = self.cols.iter().filter(|p| p.id != *dragged).collect();
        for (position, p) in others.iter().enumerate() {
            if x < p.x + p.w / 2.0 {
                return (position != original).then(|| DropSite {
                    target: p.id.clone(),
                    after: false,
                });
            }
        }
        let last = others.last()?;
        (others.len() != original).then(|| DropSite {
            target: last.id.clone(),
            after: true,
        })
    }

    /// The whole width a row takes: the columns and the space either side.
    pub fn row_width(&self) -> f32 {
        self.content + 2.0 * self.lead
    }
}
