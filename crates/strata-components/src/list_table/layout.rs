//! The column layout and widths: plain values the host keeps, shares and saves.
//!
//! [`ColumnLayout`] (order and visibility) is one value for every window and the chooser
//! (Strata's `list_column_order` and `list_hidden_columns`). [`ColumnWidths`] has one value
//! per scope (`browser_list_columns`, `chooser_list_columns`). The table reads both by
//! accessor and hands the host a new value when the person changes one; saving, sharing
//! between windows and processes, and merging are the host's.

use std::collections::{BTreeMap, BTreeSet};

use super::column::ColumnId;

/// Which columns show, in which order: every column is in `order`, and those in `hidden`
/// don't show.
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ColumnLayout {
    order: Vec<ColumnId>,
    hidden: BTreeSet<ColumnId>,
}

/// Why the layout changed: the host saves each the same way, except that a reset removes its
/// saved keys (Strata's "Reset to default columns" removes `list_column_order` and
/// `list_hidden_columns`).
///
/// Used by: C4, hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutWhy {
    /// A heading dragged to another place.
    Reorder,
    /// A column shown or hidden from the columns menu.
    Visibility,
    /// "Reset to default columns".
    Reset,
}

/// A new layout the person made, for the host to keep, save and share.
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutChange {
    /// The layout to keep.
    pub layout: ColumnLayout,
    /// What changed it.
    pub why: LayoutWhy,
}

/// Where a dragged heading drops: before `target`, or after it (only ever the last shown
/// column).
///
/// Used by: C4.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DropSite {
    /// The heading the line marks.
    pub target: ColumnId,
    /// After it rather than before.
    pub after: bool,
}

impl ColumnLayout {
    /// `order`, all shown.
    pub fn new(order: impl IntoIterator<Item = ColumnId>) -> Self {
        let mut layout = Self::default();
        for id in order {
            if !layout.order.contains(&id) {
                layout.order.push(id);
            }
        }
        layout
    }

    /// A saved layout made whole, as Strata loads one: ids `defaults` doesn't name are
    /// dropped, a repeated id keeps its first place, missing columns are appended in default
    /// order, and `primary` (Name) is never hidden.
    pub fn normalize<'a>(
        defaults: &[ColumnId],
        primary: Option<&ColumnId>,
        order: impl IntoIterator<Item = &'a str>,
        hidden: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let known = |id: &str| defaults.iter().find(|d| d.as_str() == id).cloned();
        let mut layout = Self::new(order.into_iter().filter_map(known));
        for d in defaults {
            if !layout.order.contains(d) {
                layout.order.push(d.clone());
            }
        }
        layout.hidden = hidden
            .into_iter()
            .filter_map(known)
            .filter(|id| Some(id) != primary)
            .collect();
        layout
    }

    /// Every column, shown or not, in order.
    pub fn order(&self) -> &[ColumnId] {
        &self.order
    }

    /// The hidden columns.
    pub fn hidden(&self) -> impl Iterator<Item = &ColumnId> {
        self.hidden.iter()
    }

    /// Whether `id` shows: it is in the order and not hidden.
    pub fn is_visible(&self, id: &ColumnId) -> bool {
        self.order.contains(id) && !self.hidden.contains(id)
    }

    /// The shown columns, in order.
    pub fn visible(&self) -> impl Iterator<Item = &ColumnId> {
        self.order.iter().filter(|id| !self.hidden.contains(*id))
    }

    /// The same layout with `id` shown or hidden.
    pub fn with_visible(&self, id: &ColumnId, visible: bool) -> Self {
        let mut next = self.clone();
        if visible {
            next.hidden.remove(id);
        } else if next.order.contains(id) {
            next.hidden.insert(id.clone());
        }
        next
    }

    /// The same layout with `id` moved to `site`, or `None` when that changes nothing (`id` or
    /// the target isn't in the order, or `id` would land where it is).
    pub fn moved(&self, id: &ColumnId, site: &DropSite) -> Option<Self> {
        if *id == site.target {
            return None;
        }
        let mut order = self.order.clone();
        let from = order.iter().position(|c| c == id)?;
        let moving = order.remove(from);
        let at = order.iter().position(|c| *c == site.target)? + usize::from(site.after);
        order.insert(at, moving);
        (order != self.order).then(|| Self {
            order,
            hidden: self.hidden.clone(),
        })
    }
}

/// The widths the person gave columns by hand, in base points (at Strata's default text
/// size). A column with none takes its default width, and the fill column fills.
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnWidths {
    widths: BTreeMap<ColumnId, f32>,
}

impl ColumnWidths {
    /// No column resized.
    pub fn new() -> Self {
        Self::default()
    }

    /// The width `id` was given by hand, if any.
    pub fn get(&self, id: &ColumnId) -> Option<f32> {
        self.widths.get(id).copied()
    }

    /// Every width given by hand.
    pub fn iter(&self) -> impl Iterator<Item = (&ColumnId, f32)> {
        self.widths.iter().map(|(id, w)| (id, *w))
    }

    /// The same widths with `id` at `w`.
    pub fn with(&self, id: &ColumnId, w: f32) -> Self {
        let mut next = self.clone();
        next.widths.insert(id.clone(), w);
        next
    }

    /// The same widths with `id` back to its default.
    pub fn without(&self, id: &ColumnId) -> Self {
        let mut next = self.clone();
        next.widths.remove(id);
        next
    }
}
