// SPDX-License-Identifier: MIT

//! A column's list models and the selection it mirrors into the browser.

use super::super::ColumnHits;
use crate::services::SearchItem;
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::{ViewMap, apply_selection_plan, bitset_positions};
use crate::ui::browser::entry::{entry_filter, entry_model_value};
use crate::ui::entry_list_model::EntryListModel;
use crate::ui::search_session::SearchSession;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub(super) struct ColumnModels {
    pub(super) entry_count: Rc<Cell<usize>>,
    pub(super) model: EntryListModel,
    pub(super) filter_query: Rc<RefCell<String>>,
    pub(super) show_hidden: Rc<Cell<bool>>,
    pub(super) filter: gtk::CustomFilter,
    pub(super) filtered_model: gtk::FilterListModel,
    pub(super) map: ViewMap,
    pub(super) selection: gtk::MultiSelection,
    pub(super) recursive_search_active: Rc<Cell<bool>>,
    pub(super) hits: Rc<RefCell<ColumnHits>>,
    pub(super) syncing_selection: Rc<Cell<bool>>,
    pub(super) search_results: Rc<RefCell<Vec<SearchItem>>>,
    pub(super) search_session: SearchSession,
    pub(super) search_model: gtk::StringList,
}

/// Mirrors the column's list selection into the browser's model for its depth.
struct SelectionSync {
    state: Weak<ViewState>,
    depth: usize,
    map: ViewMap,
    syncing: Rc<Cell<bool>>,
    search_active: Rc<Cell<bool>>,
    multiple_selection: Rc<Cell<bool>>,
    focused_filtered: Cell<Option<u32>>,
}

impl SelectionSync {
    fn select_only(&self, selection: &gtk::MultiSelection, position: u32) {
        self.syncing.set(true);
        selection.select_item(position, true);
        self.syncing.set(false);
    }

    fn changed(&self, selection: &gtk::MultiSelection, position: u32, count: u32) {
        if self.syncing.get() {
            return;
        }
        if self.search_active.get() {
            self.search_hits_changed(selection, position, count);
            return;
        }
        let mut filtered_positions = bitset_positions(&selection.selection());
        let focused = self.focused_position(&filtered_positions, position, count);
        let single = !self.multiple_selection.get() && filtered_positions.len() > 1;
        if single && let Some(focused) = focused {
            self.select_only(selection, focused);
            filtered_positions.clear();
            filtered_positions.push(focused);
        }
        let mapped_positions = self.map.source_positions(&filtered_positions);
        let source_positions: Vec<_> = mapped_positions
            .iter()
            .map(|(_, source_position)| *source_position)
            .collect();
        self.focused_filtered.set(focused);
        let focused_source = focused.and_then(|position| self.map.source_position(position));
        if let Some(state) = self.state.upgrade() {
            self.publish(&state, selection, &source_positions, focused_source);
        }
    }

    /// Search hits keep one selected hit without multiple selection.
    fn search_hits_changed(&self, selection: &gtk::MultiSelection, position: u32, count: u32) {
        if !self.multiple_selection.get() && selection.selection().size() > 1 {
            let focused = (position..position.saturating_add(count))
                .rev()
                .find(|position| selection.is_selected(*position));
            if let Some(focused) = focused {
                self.select_only(selection, focused);
            }
        }
        if let Some(state) = self.state.upgrade() {
            state.notify_search_selection_changed();
        }
    }

    /// The newest selected row in the changed range, else the remembered or last one.
    fn focused_position(&self, selected: &[u32], position: u32, count: u32) -> Option<u32> {
        let changed_end = position.saturating_add(count);
        selected
            .iter()
            .rev()
            .copied()
            .find(|candidate| *candidate >= position && *candidate < changed_end)
            .or_else(|| {
                self.focused_filtered
                    .get()
                    .filter(|candidate| selected.contains(candidate))
            })
            .or_else(|| selected.last().copied())
    }

    fn publish(
        &self,
        state: &ViewState,
        selection: &gtk::MultiSelection,
        source_positions: &[usize],
        focused_source: Option<usize>,
    ) {
        // Marquee and row-gesture writes own the selection. A later focus
        // echo of the cursor does not, and must not replace a committed fill.
        if crate::ui::marquee::is_updating_selection() || state.pointer_owns_selection.get() {
            state.browser.commit_selection();
        }
        state
            .browser
            .set_selection(self.depth, source_positions, focused_source);
        let model = state.browser.selected_positions(self.depth);
        if model != source_positions {
            let filtered: Vec<u32> = model
                .iter()
                .filter_map(|position| self.map.view_position(*position))
                .collect();
            self.syncing.set(true);
            apply_selection_plan(selection, selection.n_items(), &filtered);
            self.syncing.set(false);
        }
        state.refresh_destination_style();
    }
}

impl ViewState {
    fn column_entry_model(&self, depth: usize) -> EntryListModel {
        let browser_for_model = self.browser.clone();
        EntryListModel::new(Rc::new(move |position| {
            let position = position as usize;
            browser_for_model
                .with_entries(depth, position..position.saturating_add(1), |entries| {
                    entries.first().map(entry_model_value)
                })
                .flatten()
        }))
    }

    pub(super) fn column_models(self: &Rc<Self>, depth: usize) -> ColumnModels {
        let entry_count = Rc::new(Cell::new(0));
        let model = self.column_entry_model(depth);
        let filter_query = Rc::new(RefCell::new(String::new()));
        let initial_show_hidden = self
            .browser
            .column_preferences(depth)
            .map_or_else(|| self.browser.preferences().show_hidden, |p| p.show_hidden);
        let show_hidden = Rc::new(Cell::new(initial_show_hidden));
        let filter = entry_filter(show_hidden.clone(), filter_query.clone());
        let filtered_model = gtk::FilterListModel::new(Some(model.clone()), Some(filter.clone()));
        let map = ViewMap::new(
            filter_query.clone(),
            show_hidden.clone(),
            self.source_generation.clone(),
            model.clone(),
            filtered_model.clone(),
            None,
        );
        let selection = gtk::MultiSelection::new(Some(filtered_model.clone()));
        let recursive_search_active = Rc::new(Cell::new(false));
        let hits = Rc::new(RefCell::new(ColumnHits::default()));
        let syncing_selection = Rc::new(Cell::new(false));
        let sync = SelectionSync {
            state: Rc::downgrade(self),
            depth,
            map: map.clone(),
            syncing: syncing_selection.clone(),
            search_active: recursive_search_active.clone(),
            multiple_selection: self.multiple_selection.clone(),
            focused_filtered: Cell::new(None),
        };
        selection.connect_selection_changed(move |selection, position, count| {
            sync.changed(selection, position, count);
        });
        ColumnModels {
            entry_count,
            model,
            filter_query,
            show_hidden,
            filter,
            filtered_model,
            map,
            selection,
            recursive_search_active,
            hits,
            syncing_selection,
            search_results: Rc::new(RefCell::new(Vec::new())),
            search_session: SearchSession::default(),
            search_model: gtk::StringList::new(&[]),
        }
    }
}
