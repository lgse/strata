// SPDX-License-Identifier: MIT

//! Keeps each section's GTK selection and the browser's selection of a pane in step.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{
    ModeViews,
    pane::{BoundItems, BoundModeItem, Pane, PaneSection},
    source_index::{SPARSE_LOOKUP_LIMIT, SourceIndexMap, view_position_for_source},
};
use crate::{app::Browser, ui::marquee};

/// The pane a section's selection belongs to.
#[derive(Clone)]
pub(super) struct SectionOwner {
    pub(super) sections: Weak<RefCell<Vec<PaneSection>>>,
    pub(super) depth: usize,
    pub(super) source_index: SourceIndexMap,
}

pub(super) fn connect_selection(
    section: &PaneSection,
    owner: SectionOwner,
    browser: &Rc<Browser>,
    multiple_selection: Rc<Cell<bool>>,
) {
    let commit = gtk::GestureClick::new();
    commit.set_propagation_phase(gtk::PropagationPhase::Capture);
    let browser_for_commit = Rc::downgrade(browser);
    commit.connect_pressed(move |_, _, _, _| {
        if let Some(browser) = browser_for_commit.upgrade() {
            browser.commit_selection();
        }
    });
    section.view.add_controller(commit);
    let sync = SelectionSync {
        owner,
        browser: Rc::downgrade(browser),
        multiple_selection,
        syncing: section.syncing.clone(),
        view: section.view.downgrade(),
        bound_items: Rc::downgrade(&section.bound_items),
        view_model: section.view_model.clone(),
        view_index: section.view_index.clone(),
    };
    section
        .selection
        .connect_selection_changed(move |selection, position, count| {
            sync.changed(selection, position, count);
        });
}

/// Reports a section's selection changes to the browser.
struct SelectionSync {
    owner: SectionOwner,
    browser: Weak<Browser>,
    multiple_selection: Rc<Cell<bool>>,
    syncing: Rc<Cell<bool>>,
    view: glib::WeakRef<gtk::Widget>,
    bound_items: Weak<RefCell<Vec<BoundModeItem>>>,
    view_model: gtk::gio::ListModel,
    view_index: SourceIndexMap,
}

impl SelectionSync {
    fn changed(&self, selection: &gtk::MultiSelection, position: u32, count: u32) {
        if self.syncing.get() {
            return;
        }
        let (Some(sections), Some(browser)) = (self.owner.sections.upgrade(), self.browser.upgrade())
        else {
            return;
        };
        if !self.multiple_selection.get() {
            keep_single_selection(&sections, selection, position, count);
        }
        let selected = selection.selection();
        let changed = gtk::Bitset::new_range(position, count);
        changed.intersect(&selected);
        let toggled = (!changed.is_empty())
            .then(|| changed.maximum())
            .and_then(|position| self.source_of(position));
        let native_focus = self.native_focus(&selected);
        let focused = toggled
            .or(native_focus)
            .or_else(|| self.last_selected_source(&selected));
        sync_browser_selection(&sections, &browser, &self.owner, focused);
    }

    fn source_of(&self, position: u32) -> Option<usize> {
        self.view_index
            .sources_of(
                &self.owner.source_index,
                &self.view_model,
                std::iter::once(position),
            )
            .pop()
            .flatten()
    }

    /// The selected item GTK's own focus sits on.
    fn native_focus(&self, selected: &gtk::Bitset) -> Option<usize> {
        let focused = self.view.upgrade()?.root()?.focus()?;
        let bound_items = self.bound_items.upgrade()?;
        let position = focused_bound_position(&bound_items, &focused)?;
        selected
            .contains(position)
            .then_some(position)
            .and_then(|position| self.source_of(position))
    }

    fn last_selected_source(&self, selected: &gtk::Bitset) -> Option<usize> {
        let (mut lower, last) = gtk::BitsetIter::init_last(selected)?;
        std::iter::once(last)
            .chain(std::iter::from_fn(|| lower.previous()))
            .find_map(|position| self.source_of(position))
    }
}

fn focused_bound_position(bound_items: &BoundItems, focused: &gtk::Widget) -> Option<u32> {
    bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let owns_focus = widget == *focused
            || focused.is_ancestor(&widget)
            || widget.parent().as_ref() == Some(focused);
        owns_focus
            .then(|| bound.item.upgrade().map(|item| item.position()))
            .flatten()
    })
}

/// Without multiple selection, only the changed item stays selected, in its own section.
fn keep_single_selection(
    sections: &RefCell<Vec<PaneSection>>,
    selection: &gtk::MultiSelection,
    position: u32,
    count: u32,
) {
    let positions = bitset_positions(&selection.selection());
    let end = position.saturating_add(count) as usize;
    let Some(focused) = positions
        .iter()
        .rev()
        .copied()
        .find(|candidate| (position as usize..end).contains(candidate))
        .or_else(|| positions.last().copied())
    else {
        return;
    };
    for other in sections.borrow().iter() {
        other.syncing.set(true);
        if other.selection == *selection {
            other.selection.select_item(focused as u32, true);
        } else {
            other.selection.unselect_all();
        }
        other.syncing.set(false);
    }
}

pub(super) fn select_all(pane: &Pane) {
    for section in pane.item_sections() {
        section.syncing.set(true);
        section.selection.select_all();
        section.syncing.set(false);
    }
}

pub(super) fn set_selections(pane: &Pane, positions: &[usize]) {
    let all_selected = positions.len() == pane.model.n_items() as usize;
    for section in pane.item_sections() {
        let total_items = section.selection.n_items();
        let shows_every_entry = section.view_model.n_items() == pane.model.n_items();
        let selected = if all_selected && shows_every_entry {
            gtk::Bitset::new_range(0, total_items)
        } else {
            section_selection(pane, &section, positions)
        };
        section.syncing.set(true);
        section
            .selection
            .set_selection(&selected, &gtk::Bitset::new_range(0, total_items));
        section.syncing.set(false);
    }
}

/// The view positions in `section` of the directory entries at `positions`.
fn section_selection(pane: &Pane, section: &PaneSection, positions: &[usize]) -> gtk::Bitset {
    if positions.len() > SPARSE_LOOKUP_LIMIT {
        section.view_index.ensure_current();
    }
    let selected = gtk::Bitset::new_empty();
    for position in positions
        .iter()
        .filter_map(|position| section.source_to_view(&pane.model, *position))
    {
        selected.add(position);
    }
    selected
}

pub(super) fn reset_native_range_origin(pane: &Pane, source_position: usize) {
    for section in pane.item_sections() {
        let Some(position) =
            view_position_for_source(&pane.model, Some(&section.view_model), source_position)
        else {
            continue;
        };
        // MultiSelection bits do not move GtkListView/GridView's Shift range
        // origin; list.select-item does.
        section.syncing.set(true);
        section
            .view
            .activate_action(
                "list.select-item",
                Some(&(position, false, false).to_variant()),
            )
            .expect("ListView and GridView expose list.select-item");
        section.syncing.set(false);
    }
}

pub(super) fn scroll_pane_to_source(pane: &Pane, source_position: usize) {
    for section in pane.item_sections() {
        let Some(position) =
            view_position_for_source(&pane.model, Some(&section.view_model), source_position)
        else {
            continue;
        };
        if position >= section.view_model.n_items() {
            continue;
        }
        scroll_collection_to(&section.view, position);
        return;
    }
}

pub(super) fn scroll_collection_to(view: &gtk::Widget, position: u32) {
    crate::ui::browser::scroll_collection_when_allocated(view, position);
}

pub(super) fn selected_source_positions(
    source_index: &SourceIndexMap,
    section: &PaneSection,
) -> Vec<usize> {
    let selected = section.selection.selection();
    let positions = gtk::BitsetIter::init_first(&selected)
        .into_iter()
        .flat_map(|(rest, first)| std::iter::once(first).chain(rest));
    section
        .view_index
        .sources_of(source_index, &section.view_model, positions)
        .into_iter()
        .flatten()
        .collect()
}

/// Reports the selection of every section in a pane, so a grouped view keeps items
/// picked in other groups selected.
fn sync_browser_selection(
    sections: &RefCell<Vec<PaneSection>>,
    browser: &Browser,
    owner: &SectionOwner,
    focused: Option<usize>,
) {
    let (depth, source_index) = (owner.depth, &owner.source_index);
    let mut positions: Vec<usize> = {
        let sections = sections.borrow();
        sections
            .iter()
            .flat_map(|section| selected_source_positions(source_index, section))
            .collect()
    };
    positions.sort_unstable();
    positions.dedup();
    let focused = focused.or_else(|| positions.last().copied());
    if marquee::is_updating_selection() {
        browser.commit_selection();
    }
    browser.set_selection(depth, &positions, focused);
    let model = browser.selected_positions(depth);
    if model == positions {
        return;
    }
    for section in sections.borrow().iter() {
        select_model_positions(section, source_index, &model);
    }
}

/// Selects exactly the items of `section` whose sources the browser has selected.
fn select_model_positions(section: &PaneSection, source_index: &SourceIndexMap, model: &[usize]) {
    let selected = gtk::Bitset::new_empty();
    let count = section.view_model.n_items();
    let sources = section
        .view_index
        .sources_of(source_index, &section.view_model, 0..count);
    for (view_position, source) in (0..count).zip(sources) {
        if source.is_some_and(|source| model.binary_search(&source).is_ok()) {
            selected.add(view_position);
        }
    }
    section.syncing.set(true);
    section
        .selection
        .set_selection(&selected, &gtk::Bitset::new_range(0, count));
    section.syncing.set(false);
}

pub(super) fn bitset_positions(bitset: &gtk::Bitset) -> Vec<usize> {
    let Some((iterator, first)) = gtk::BitsetIter::init_first(bitset) else {
        return Vec::new();
    };
    std::iter::once(first)
        .chain(iterator)
        .map(|position| position as usize)
        .collect()
}

impl ModeViews {
    pub fn selected_positions(&self) -> Option<(usize, Vec<usize>)> {
        let pane = self.single_pane()?;
        let mut positions: Vec<usize> = pane
            .item_sections()
            .iter()
            .flat_map(|section| selected_source_positions(&pane.source_index, section))
            .collect();
        positions.sort_unstable();
        positions.dedup();
        Some((pane.depth, positions))
    }

    pub fn resume_native_selection(&self) -> bool {
        let Some((depth, focused, _)) = self.browser.focused_item() else {
            return false;
        };
        if !self.browser.selected_positions(depth).is_empty() {
            return false;
        }
        // Seed GTK's empty selection before the arrow moves it, without scheduling
        // a focus restore that would undo the native move after key dispatch.
        for pane in self.panes_at(depth) {
            set_selections(pane, &[focused]);
            reset_native_range_origin(pane, focused);
        }
        self.browser.set_selection(depth, &[focused], Some(focused));
        self.browser.set_selection_anchor(depth, focused);
        true
    }
}
