// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use crate::model::Location;

use super::{
    ColumnState, NavigationState, VisualKind, VisualRange, adopt_selected_locations,
    visible_positions,
};

impl NavigationState {
    /// A load cursor is not included in the range's base fill.
    pub fn start_visual(
        &mut self,
        kind: VisualKind,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        self.visual = None;
        let depth = self
            .active_column
            .or_else(|| self.columns.len().checked_sub(1))?;
        let column = self.columns.get_mut(depth)?;
        let order = displayed_order(column, order);
        let cursor = column
            .selected
            .filter(|position| order.contains(position))?;
        column.drop_load_intents();
        if column.load_cursor.is_some() {
            column.selected_locations.clear();
            column.load_cursor = None;
        }
        self.visual = Some(VisualRange {
            depth,
            directory: column.location.clone(),
            anchor: column.entries[cursor].location.clone(),
            kind,
            base: column.selected_locations.materialize(&column.entries),
            toggled: HashSet::new(),
        });
        self.refresh_visual(Some(&order))
    }

    /// A range whose pane, anchor, or cursor disappears keeps its last fill.
    pub fn refresh_visual(
        &mut self,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        let range = self.visual.as_ref()?;
        let depth = range.depth;
        let Some(column) = self
            .columns
            .get_mut(depth)
            .filter(|column| column.location == range.directory)
            .filter(|_| self.active_column == Some(depth))
        else {
            self.visual = None;
            return None;
        };
        let order = displayed_order(column, order);
        let index_of = |location: &Location| {
            order
                .iter()
                .position(|position| &column.entries[*position].location == location)
        };
        let anchor = index_of(&range.anchor);
        let cursor = column
            .selected
            .and_then(|cursor| order.iter().position(|position| *position == cursor));
        let (Some(anchor), Some(cursor)) = (anchor, cursor) else {
            self.visual = None;
            return None;
        };
        let span = order[anchor.min(cursor)..=anchor.max(cursor)]
            .iter()
            .map(|position| &column.entries[*position].location);
        let mut fill = range.base.clone();
        match range.kind {
            VisualKind::Select => fill.extend(span.cloned()),
            VisualKind::Unset => span.for_each(|location| {
                fill.remove(location);
            }),
        }
        for location in &range.toggled {
            if !fill.remove(location) {
                fill.insert(location.clone());
            }
        }
        adopt_selected_locations(column, fill, true);
        let focused = order[cursor];
        Some((depth, focused, column.selected_positions().to_vec()))
    }

    pub fn toggle_visual_cursor(
        &mut self,
        order: Option<&[usize]>,
    ) -> Option<(usize, usize, Vec<usize>)> {
        let range = self.visual.as_mut()?;
        let location = self
            .columns
            .get(range.depth)
            .and_then(|column| {
                column
                    .selected
                    .and_then(|cursor| column.entries.get(cursor))
            })
            .map(|entry| entry.location.clone());
        if let Some(location) = location
            && !range.toggled.remove(&location)
        {
            range.toggled.insert(location);
        }
        self.refresh_visual(order)
    }

    pub fn take_visual(&mut self) -> Option<VisualRange> {
        self.visual.take()
    }

    pub fn restore_visual(&mut self, range: Option<VisualRange>) {
        self.visual = range;
    }

    pub fn leave_visual(&mut self) -> bool {
        self.visual.take().is_some()
    }

    pub fn visual_kind(&self) -> Option<VisualKind> {
        self.live_range().map(|range| range.kind)
    }

    pub(super) fn live_range(&self) -> Option<&VisualRange> {
        let range = self.visual.as_ref()?;
        let column = self.columns.get(range.depth)?;
        (column.location == range.directory
            && self.active_column == Some(range.depth)
            && column
                .entries
                .iter()
                .any(|entry| entry.location == range.anchor))
        .then_some(range)
    }
}

pub(super) fn displayed_order(column: &ColumnState, order: Option<&[usize]>) -> Vec<usize> {
    match order {
        Some(order)
            if !order.is_empty()
                && order
                    .iter()
                    .all(|position| *position < column.entries.len()) =>
        {
            order.to_vec()
        }
        _ => visible_positions(column),
    }
}
