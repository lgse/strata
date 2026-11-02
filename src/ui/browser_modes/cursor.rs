// SPDX-License-Identifier: MIT

//! Where the browser's cursor sits in a pane's views, and how keyboard paging and
//! reveals move through them.

use std::cmp::Ordering;

use gtk::prelude::*;

use super::{
    ModeViews,
    pane::{Pane, PaneSection},
    source_index::{source_position_for_view, view_position_for_source},
};
use crate::model::Location;

impl ModeViews {
    pub(in crate::ui) fn page_target(
        &self,
        depth: usize,
        direction: i32,
        page: usize,
    ) -> Option<usize> {
        let pane = self.visible_pane(depth)?;
        let sections = pane.item_sections();
        let count: usize = sections
            .iter()
            .map(|section| section.view_model.n_items() as usize)
            .sum();
        let last = count.checked_sub(1)?;
        let focused = self.browser.focused_item().filter(|(d, _, _)| *d == depth);
        let current = match focused {
            // A filtered-out cursor still needs the existing visual-order fallback.
            Some((_, source, _)) => Some(visual_position(pane, &sections, source)?),
            None => None,
        };
        let target = page_destination(current, direction.cmp(&0), page, last)?;
        source_at_visual_position(pane, &sections, target)
    }

    pub fn visual_order(&self, depth: usize) -> Vec<usize> {
        let Some(pane) = self.visible_pane(depth) else {
            return Vec::new();
        };
        pane.item_sections()
            .into_iter()
            .flat_map(|section| {
                (0..section.view_model.n_items())
                    .filter_map(|position| {
                        source_position_for_view(
                            &pane.source_index,
                            Some(&section.view_model),
                            position,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn reveal_selected_entry(&self, depth: usize, source_position: usize) {
        let target = self
            .visible_panes()
            .into_iter()
            .filter(|pane| pane.depth == depth)
            .find_map(|pane| pane.view_position_of(source_position));
        if let Some((section, position)) = target {
            crate::ui::browser::reveal_collection_after_layout(
                &section.view,
                position,
                section.visit.clone(),
            );
        }
    }

    pub(in crate::ui) fn cursor_view(
        &self,
        depth: usize,
        source: usize,
    ) -> Option<(gtk::Widget, u32)> {
        self.visible_pane(depth)?.cursor_view(source)
    }

    pub fn view_position_in(&self, view: &gtk::Widget, depth: usize, source: usize) -> Option<u32> {
        let pane = self.visible_pane(depth)?;
        pane.item_sections()
            .into_iter()
            .find(|section| section.view == *view)
            .and_then(|section| section.source_to_view(&pane.model, source))
    }

    pub(in crate::ui) fn keyboard_peek_target(&self) -> Option<(gtk::Widget, usize, Location)> {
        if let Some(pane) = self.single_pane()
            && pane.search.selected_entries().is_some()
        {
            let (widget, entry) = pane.search.selected_anchor()?;
            return entry
                .is_directory()
                .then_some((widget, pane.depth, entry.location));
        }
        let (depth, position, entry) = self.browser.focused_item()?;
        if !entry.is_directory() {
            return None;
        }
        let pane = self.mode_pane(depth)?;
        let widget = pane.item_sections().into_iter().find_map(|section| {
            let view_position =
                view_position_for_source(&pane.model, Some(&section.view_model), position)?;
            section.item_widget(view_position)
        })?;
        Some((widget, depth, entry.location))
    }

    fn visible_pane(&self, depth: usize) -> Option<&Pane> {
        self.visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)
    }
}

/// The position of a directory entry across the pane's sections, in visual order.
fn visual_position(pane: &Pane, sections: &[PaneSection], source: usize) -> Option<usize> {
    let mut offset = 0;
    sections.iter().find_map(|section| {
        let position = section.source_to_view(&pane.model, source);
        let start = offset;
        offset += section.view_model.n_items() as usize;
        position.map(|position| start + position as usize)
    })
}

/// Where a page move from `current` lands among `last + 1` visual positions.
fn page_destination(
    current: Option<usize>,
    direction: Ordering,
    page: usize,
    last: usize,
) -> Option<usize> {
    let full_jump = page == usize::MAX;
    let target = match (current, direction) {
        (_, Ordering::Equal) => return None,
        // A full jump is absolute even without a cursor, as in `NavigationState`.
        (_, Ordering::Less) if full_jump => 0,
        (_, Ordering::Greater) if full_jump => last,
        (None, Ordering::Less) => last,
        (None, _) => 0,
        (Some(current), Ordering::Less) => current.saturating_sub(page.max(1)),
        (Some(current), _) => current.saturating_add(page.max(1)).min(last),
    };
    Some(target)
}

fn source_at_visual_position(
    pane: &Pane,
    sections: &[PaneSection],
    mut target: usize,
) -> Option<usize> {
    for section in sections {
        let count = section.view_model.n_items() as usize;
        if target < count {
            return pane
                .source_index
                .of_view_position(&section.view_model, target as u32);
        }
        target -= count;
    }
    None
}
