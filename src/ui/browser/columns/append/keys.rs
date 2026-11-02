// SPDX-License-Identifier: MIT

//! Keyboard and activation routes between a column's filter field and its listing.

use super::header::ColumnHeader;
use super::models::ColumnModels;
use crate::app::Browser;
use crate::services::SearchItem;
use crate::ui::browser::collection::{
    ViewMap, activate_recursive_search_result, bitset_positions, recursive_search_activation_key,
    search_result_navigation_position,
};
use crate::ui::browser::{BrowserView, ViewState};
use crate::ui::enter_target::EnterTarget;
use crate::ui::keys::{
    self, KeyPress,
    bindings::{DOWN, ENTER, UP},
};
use gtk::glib::{self, Propagation};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

fn propagation(handled: bool) -> Propagation {
    if handled {
        Propagation::Stop
    } else {
        Propagation::Proceed
    }
}

fn filter_active(search_active: &Cell<bool>, query: &RefCell<String>) -> bool {
    search_active.get() || !query.borrow().trim().is_empty()
}

fn note_keyboard_navigation(state: &Weak<ViewState>) {
    if let Some(state) = state.upgrade() {
        BrowserView { state }.keyboard_navigation();
    }
}

/// Down and Enter in the filter field pick from the filtered listing or the search hits.
struct FilterNavigation {
    state: Weak<ViewState>,
    browser: Weak<Browser>,
    depth: usize,
    map: ViewMap,
    query: Rc<RefCell<String>>,
    search_active: Rc<Cell<bool>>,
    results: Rc<RefCell<Vec<SearchItem>>>,
    selection: gtk::MultiSelection,
    syncing: Rc<Cell<bool>>,
    list: gtk::ListView,
    enter: EnterTarget,
}

impl FilterNavigation {
    fn pressed(&self, press: &KeyPress) -> Propagation {
        if !filter_active(&self.search_active, &self.query) {
            return Propagation::Proceed;
        }
        if press.is(UP) {
            return Propagation::Stop;
        }
        let activate = recursive_search_activation_key(press);
        if !activate && !press.is(DOWN) {
            return Propagation::Proceed;
        }
        // Right keeps needing a pick: in the field it otherwise moves the caret.
        let to_top = !activate || press.is(ENTER);
        let current = if to_top && self.enter.offers_top() {
            Some(0)
        } else {
            bitset_positions(&self.selection.selection())
                .last()
                .copied()
        };
        if activate {
            return propagation(self.activate(current));
        }
        self.move_into_list(current)
    }

    fn activate(&self, current: Option<u32>) -> bool {
        if self.search_active.get() {
            return current.is_some_and(|position| {
                activate_recursive_search_result(&self.browser, &self.results, position)
            });
        }
        let Some(position) = current else {
            return false;
        };
        let Some(browser) = self.browser.upgrade() else {
            return false;
        };
        let Some(source_position) = self.map.source_position(position) else {
            return false;
        };
        browser.activate(self.depth, source_position);
        true
    }

    fn move_into_list(&self, current: Option<u32>) -> Propagation {
        let Some(next) = current
            .or_else(|| search_result_navigation_position(None, self.selection.n_items(), 1))
        else {
            return Propagation::Stop;
        };
        let is_recursive = self.search_active.get();
        if is_recursive {
            self.syncing.set(true);
        }
        self.list.grab_focus();
        self.selection.select_item(next, true);
        if is_recursive {
            self.syncing.set(false);
        }
        self.enter.pick();
        self.list.scroll_to(next, gtk::ListScrollFlags::FOCUS, None);
        note_keyboard_navigation(&self.state);
        Propagation::Stop
    }
}

/// Up from the first filtered row returns to the filter field.
struct ReturnToFilter {
    state: Weak<ViewState>,
    query: Rc<RefCell<String>>,
    search_active: Rc<Cell<bool>>,
    selection: gtk::MultiSelection,
    entry: glib::WeakRef<gtk::Entry>,
}

fn focus_in_popover(controller: &gtk::EventControllerKey) -> bool {
    controller
        .widget()
        .and_then(|widget| widget.root())
        .and_then(|root| root.focus())
        .and_then(|focus| focus.ancestor(gtk::Popover::static_type()))
        .is_some()
}

impl ReturnToFilter {
    fn pressed(&self, controller: &gtk::EventControllerKey, press: &KeyPress) -> Propagation {
        let press = &press.untyped();
        if !filter_active(&self.search_active, &self.query) || !press.is_any(&[UP, DOWN]) {
            return Propagation::Proceed;
        }
        if focus_in_popover(controller) {
            return Propagation::Proceed;
        }
        note_keyboard_navigation(&self.state);
        if !press.is(UP) || bitset_positions(&self.selection.selection()).as_slice() != [0] {
            return Propagation::Proceed;
        }
        self.entry
            .upgrade()
            .filter(|entry| entry.grab_focus_without_selecting())
            .map_or(Propagation::Proceed, |_| Propagation::Stop)
    }
}

/// Activating a listing row opens its entry or search hit.
struct ListActivation {
    state: Weak<ViewState>,
    browser: Weak<Browser>,
    depth: usize,
    map: ViewMap,
    search_active: Rc<Cell<bool>>,
    results: Rc<RefCell<Vec<SearchItem>>>,
}

impl ListActivation {
    fn activated(&self, position: u32) {
        if let Some(state) = self.state.upgrade() {
            state.cancel_click_rename();
        }
        if self.search_active.get() {
            activate_recursive_search_result(&self.browser, &self.results, position);
            return;
        }
        if let (Some(state), Some(source_position)) =
            (self.state.upgrade(), self.map.source_position(position))
        {
            state.activate_column_entry(self.depth, source_position);
        }
    }
}

impl ViewState {
    pub(super) fn install_filter_navigation(
        self: &Rc<Self>,
        depth: usize,
        header: &ColumnHeader,
        models: &ColumnModels,
        list: &gtk::ListView,
    ) {
        let search_navigation = gtk::EventControllerKey::new();
        search_navigation.set_propagation_phase(gtk::PropagationPhase::Capture);
        let navigation = FilterNavigation {
            state: Rc::downgrade(self),
            browser: Rc::downgrade(&self.browser),
            depth,
            map: models.map.clone(),
            query: models.filter_query.clone(),
            search_active: models.recursive_search_active.clone(),
            results: models.search_results.clone(),
            selection: models.selection.clone(),
            syncing: models.syncing_selection.clone(),
            list: list.clone(),
            enter: header.enter_target.clone(),
        };
        keys::on_press(&search_navigation, move |_, press| {
            navigation.pressed(press)
        });
        header.filter_entry.add_controller(search_navigation);
    }

    pub(super) fn install_return_to_filter(
        self: &Rc<Self>,
        header: &ColumnHeader,
        models: &ColumnModels,
        list: &gtk::ListView,
    ) {
        let return_to_filter = gtk::EventControllerKey::new();
        return_to_filter.set_propagation_phase(gtk::PropagationPhase::Capture);
        let route = ReturnToFilter {
            state: Rc::downgrade(self),
            query: models.filter_query.clone(),
            search_active: models.recursive_search_active.clone(),
            selection: models.selection.clone(),
            entry: header.filter_entry.downgrade(),
        };
        keys::on_press(&return_to_filter, move |controller, press| {
            route.pressed(controller, press)
        });
        list.add_controller(return_to_filter);
    }

    pub(super) fn connect_list_activation(
        self: &Rc<Self>,
        depth: usize,
        models: &ColumnModels,
        list: &gtk::ListView,
    ) {
        let activation = ListActivation {
            state: Rc::downgrade(self),
            browser: Rc::downgrade(&self.browser),
            depth,
            map: models.map.clone(),
            search_active: models.recursive_search_active.clone(),
            results: models.search_results.clone(),
        };
        list.connect_activate(move |_, position| activation.activated(position));
    }
}
