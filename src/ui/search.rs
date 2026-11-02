// SPDX-License-Identifier: MIT

mod panel;
mod results;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use gtk::{glib, prelude::*};

use super::keys::{
    self, KeyPress,
    bindings::{DOWN, ENTER_ANY_SHIFT, ESCAPE_ANY, OPEN_CONTAINING_FOLDER, UP},
};
use crate::services::{
    NavigationHistory, SearchCoverage, SearchEvent, SearchHandle, SearchItem,
    index_trees_with_exclusions,
};
use panel::SearchWidgets;
use results::{refresh_visible_thumbnails, render_results, scroll_row_into_view};

const MAX_RESULT_UPDATES_PER_FRAME: usize = 8;

#[derive(Clone)]
pub struct SearchDialog {
    state: Rc<SearchState>,
}

struct SearchState {
    // Keep the shared provider alive when this dialog is hosted without a browser window.
    _themes: Rc<super::theme::ThemeManager>,
    preferences: Rc<super::preferences::PreferenceManager>,
    layer: gtk::Box,
    field: gtk::Entry,
    indexing_spinner: gtk::Spinner,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    results: gtk::Stack,
    status: gtk::Label,
    truncated_hint: gtk::Box,
    visible_results: RefCell<Vec<SearchItem>>,
    positions: Rc<RefCell<HashMap<gtk::ListBoxRow, usize>>>,
    requested_thumbnails: RefCell<HashSet<PathBuf>>,
    rendered_query: RefCell<String>,
    search: RefCell<Option<SearchHandle>>,
    history: RefCell<Option<Rc<NavigationHistory>>>,
    generation: Cell<u64>,
    interaction_revision: Cell<u64>,
    navigation_started: Cell<bool>,
    reconciling_results: Cell<bool>,
    activate: Rc<dyn Fn(SearchItem)>,
    reveal: Rc<dyn Fn(SearchItem)>,
    context_menu: RefCell<Option<gtk::Popover>>,
    dismiss: Rc<dyn Fn()>,
    /// Cleared when a result is chosen, so focus follows the browser, not the opener.
    focus_restore: Rc<Cell<bool>>,
}

impl SearchDialog {
    pub fn new(
        activate: Rc<dyn Fn(SearchItem)>,
        reveal: Rc<dyn Fn(SearchItem)>,
        dismiss: Rc<dyn Fn()>,
    ) -> Self {
        let themes = super::theme::ThemeManager::shared();
        let preferences = super::preferences::PreferenceManager::shared();
        let widgets = SearchWidgets::build();
        let focus_restore =
            super::modal::remember_persistent_modal_focus(widgets.layer.upcast_ref());
        let panel = widgets.panel;
        let state = Rc::new(SearchState {
            _themes: themes,
            preferences,
            layer: widgets.layer,
            field: widgets.field,
            indexing_spinner: widgets.indexing_spinner,
            list: widgets.list,
            scroller: widgets.scroller,
            results: widgets.results,
            status: widgets.status,
            truncated_hint: widgets.truncated_hint,
            visible_results: RefCell::new(Vec::new()),
            positions: widgets.positions,
            requested_thumbnails: RefCell::new(HashSet::new()),
            rendered_query: RefCell::new(String::new()),
            search: RefCell::new(None),
            history: RefCell::new(None),
            generation: Cell::new(0),
            interaction_revision: Cell::new(0),
            navigation_started: Cell::new(false),
            reconciling_results: Cell::new(false),
            activate,
            reveal,
            context_menu: RefCell::new(None),
            dismiss,
            focus_restore,
        });

        connect_field_and_list(&state);
        install_keys(&state);
        install_backdrop_click(&state, &panel);
        install_wheel(&state);
        follow_scrolling(&state);
        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.layer.clone().upcast()
    }

    pub fn show(&self, roots: Vec<PathBuf>, show_hidden: bool) {
        self.state.generation.set(self.state.generation.get() + 1);
        let generation = self.state.generation.get();
        self.state.search.borrow_mut().take();
        self.state.history.borrow_mut().take();
        reset_for_search(&self.state, &roots);
        if roots.is_empty() {
            show_no_locations(&self.state);
            return;
        }
        start_search(&self.state, roots, show_hidden, generation);
    }

    pub(crate) fn show_history(&self, history: Rc<NavigationHistory>) {
        self.state.generation.set(self.state.generation.get() + 1);
        self.state.search.borrow_mut().take();
        self.state.history.replace(Some(history));
        self.state
            .field
            .set_placeholder_text(Some(&crate::i18n::tr("Jump to a folder…")));
        crate::ui::accessibility::set_description(
            &self.state.field,
            Some(&crate::i18n::tr("Folders previously visited in Strata")),
        );
        self.state.field.set_sensitive(true);
        clear_results(&self.state);
        self.state.field.set_text("");
        self.state.status.set_visible(true);
        self.state.truncated_hint.set_visible(false);
        self.state.indexing_spinner.stop();
        self.state.indexing_spinner.set_visible(false);
        self.state.layer.set_visible(true);
        super::browser::animate_in(&self.state.layer);
        self.state.field.grab_focus_without_selecting();
        begin_query(&self.state, "");
    }

    pub fn hide(&self) {
        hide(&self.state);
    }

    pub fn is_visible(&self) -> bool {
        self.state.layer.is_visible()
    }
}

fn connect_field_and_list(state: &Rc<SearchState>) {
    let changed = Rc::downgrade(state);
    state.field.connect_changed(move |field| {
        if let Some(state) = changed.upgrade() {
            begin_query(&state, &field.text());
        }
    });
    // Enter reaches the field only after an input method commits and passes
    // it back; otherwise the key handler below opens the result.
    let entered = Rc::downgrade(state);
    state.field.connect_activate(move |_| {
        if let Some(state) = entered.upgrade() {
            activate_selected(&state);
        }
    });
    let activated = Rc::downgrade(state);
    state.list.connect_row_activated(move |_, row| {
        if let Some(state) = activated.upgrade() {
            activate_position(&state, row.index());
        }
    });
}

fn install_keys(state: &Rc<SearchState>) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let keyed = Rc::downgrade(state);
    keys::on_press(&keys, move |_, press| {
        let Some(state) = keyed.upgrade() else {
            return glib::Propagation::Proceed;
        };
        handle_key(&state, press)
    });
    state.layer.add_controller(keys);
}

fn handle_key(state: &Rc<SearchState>, press: &KeyPress) -> glib::Propagation {
    record_interaction(state);
    let press = &press.untyped();
    if press.is(ESCAPE_ANY) {
        hide(state);
        return glib::Propagation::Stop;
    }
    if press.is(OPEN_CONTAINING_FOLDER) {
        reveal_selected(state);
        return glib::Propagation::Stop;
    }
    if press.is_any(&[DOWN, UP]) {
        move_selection(state, if press.is(DOWN) { 1 } else { -1 });
        return glib::Propagation::Stop;
    }
    if press.is(ENTER_ANY_SHIFT) && activate_selected(state) {
        return glib::Propagation::Stop;
    }
    glib::Propagation::Proceed
}

fn reveal_selected(state: &SearchState) {
    if let Some(item) = state.list.selected_row().and_then(|row| {
        state
            .visible_results
            .borrow()
            .get(row.index() as usize)
            .cloned()
    }) {
        reveal_result(state, item);
    }
}

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
fn install_backdrop_click(state: &Rc<SearchState>, panel: &gtk::Box) {
    let click_state = Rc::downgrade(state);
    let click_panel = panel.clone();
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed(move |_, _, x, y| {
        let Some(state) = click_state.upgrade() else {
            return;
        };
        record_interaction(&state);
        let on_panel = click_panel
            .translate_coordinates(&state.layer, 0.0, 0.0)
            .is_some_and(|(px, py)| {
                let alloc = click_panel.allocation();
                x >= px
                    && x < px + alloc.width() as f64
                    && y >= py
                    && y < py + alloc.height() as f64
            });
        if !on_panel {
            hide(&state);
        }
    });
    state.layer.add_controller(click);
}

fn install_wheel(state: &Rc<SearchState>) {
    let wheel_state = Rc::downgrade(state);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    wheel.connect_scroll(move |_, _, _| {
        if let Some(state) = wheel_state.upgrade() {
            record_interaction(&state);
        }
        glib::Propagation::Proceed
    });
    state.layer.add_controller(wheel);
}

fn follow_scrolling(state: &Rc<SearchState>) {
    let adjustment = state.scroller.vadjustment();
    let changed = Rc::downgrade(state);
    adjustment.connect_changed(move |_| {
        if let Some(state) = changed.upgrade() {
            refresh_unless_reconciling(&state);
        }
    });
    let scrolled = Rc::downgrade(state);
    adjustment.connect_value_changed(move |_| {
        if let Some(state) = scrolled.upgrade() {
            refresh_unless_reconciling(&state);
        }
    });
}

fn refresh_unless_reconciling(state: &SearchState) {
    if !state.reconciling_results.get() {
        refresh_visible_thumbnails(state);
    }
}

fn reset_for_search(state: &SearchState, roots: &[PathBuf]) {
    state
        .field
        .set_placeholder_text(Some(&crate::i18n::tr("Search files and folders…")));
    let locations = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    crate::ui::accessibility::set_description(
        &state.field,
        Some(&rust_i18n::t!(
            "Search locations:\n%{locations}\nRemote shares are not included.",
            locations = locations
        )),
    );
    state.field.set_sensitive(!roots.is_empty());
    clear_results(state);
    state.results.set_visible_child_name("status");
    state.field.set_text("");
    state.status.set_visible(true);
    state.status.set_text(&crate::i18n::tr(
        "Type to search Home and mounted local drives",
    ));
    state.truncated_hint.set_visible(false);
    show_indexing(state, true);
    state.layer.set_visible(true);
    super::browser::animate_in(&state.layer);
    state.field.grab_focus_without_selecting();
}

fn show_no_locations(state: &SearchState) {
    state
        .status
        .set_text(&crate::i18n::tr("No local search locations available."));
    show_indexing(state, false);
    state.layer.grab_focus();
}

fn show_indexing(state: &SearchState, indexing: bool) {
    if indexing {
        state.indexing_spinner.set_visible(true);
        state.indexing_spinner.start();
    } else {
        state.indexing_spinner.stop();
        state.indexing_spinner.set_visible(false);
    }
}

fn start_search(state: &Rc<SearchState>, roots: Vec<PathBuf>, show_hidden: bool, generation: u64) {
    let exclusions = state.preferences.search_exclusions();
    let limits = state.preferences.search_limits();
    let (handle, receiver) = index_trees_with_exclusions(roots, show_hidden, exclusions, limits);
    state.search.replace(Some(handle));
    let weak = Rc::downgrade(state);
    let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(state) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !state.layer.is_visible() || state.generation.get() != generation {
            return glib::ControlFlow::Break;
        }
        let Ok(latest) = latest_event(&receiver) else {
            return glib::ControlFlow::Break;
        };
        if let Some(event) = latest {
            apply_search_event(&state, event);
        }
        glib::ControlFlow::Continue
    });
}

/// The newest of up to a frame's worth of queued events.
fn latest_event(receiver: &Receiver<SearchEvent>) -> Result<Option<SearchEvent>, TryRecvError> {
    let mut latest = None;
    for _ in 0..MAX_RESULT_UPDATES_PER_FRAME {
        match receiver.try_recv() {
            Ok(event) => latest = Some(event),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return Err(TryRecvError::Disconnected),
        }
    }
    Ok(latest)
}

fn apply_search_event(state: &Rc<SearchState>, event: SearchEvent) {
    let SearchEvent::Results {
        query,
        items,
        indexing,
        coverage,
        ..
    } = event;
    show_indexing(state, indexing);
    let current = query == state.field.text().trim();
    if current {
        show_coverage(&state.truncated_hint, coverage);
    }
    if current && !query.is_empty() {
        render_results(state, items, indexing, coverage);
    }
}

fn begin_query(state: &Rc<SearchState>, query: &str) {
    record_interaction(state);
    state.navigation_started.set(false);
    if let Some(history) = state.history.borrow().clone() {
        render_results(
            state,
            history.search(query),
            false,
            SearchCoverage::default(),
        );
        return;
    }
    if query.trim().is_empty() {
        clear_results(state);
        state.results.set_visible_child_name("status");
        state.status.set_text(
            &crate::i18n::tr("Type to search Home and mounted local drives\nFuzzy matching · try a name or path fragment"),
        );
    } else if state.visible_results.borrow().is_empty() {
        state.results.set_visible_child_name("status");
        state.status.set_text(&crate::i18n::tr("Searching…"));
    }
    if let Some(search) = state.search.borrow().as_ref() {
        search.query(query);
    }
}

fn close_result_menu(state: &SearchState) {
    if let Some(popover) = state.context_menu.take() {
        popover.popdown();
    }
}

fn reveal_result(state: &SearchState, item: SearchItem) {
    skip_focus_restore(state);
    let reveal = state.reveal.clone();
    hide_then(state, move || reveal(item));
}

fn contains_keyboard_focus(widget: &gtk::Widget) -> bool {
    widget
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| gtk::prelude::GtkWindowExt::focus(&window))
        .is_some_and(|focused| focused == *widget || focused.is_ancestor(widget))
}

fn record_interaction(state: &SearchState) {
    state
        .interaction_revision
        .set(state.interaction_revision.get() + 1);
}

fn move_selection(state: &SearchState, direction: i32) {
    record_interaction(state);
    if !contains_keyboard_focus(state.field.upcast_ref()) {
        state.field.grab_focus_without_selecting();
    }
    let count = state.visible_results.borrow().len() as i32;
    if count == 0 {
        return;
    }

    state.navigation_started.set(true);
    let next = state
        .list
        .selected_row()
        .map_or(0, |row| (row.index() + direction).clamp(0, count - 1));
    if let Some(row) = state.list.row_at_index(next) {
        state.list.select_row(Some(&row));
        scroll_row_into_view(state, &row);
    }
}

fn activate_selected(state: &Rc<SearchState>) -> bool {
    let Some(row) = state.list.selected_row() else {
        return false;
    };
    activate_position(state, row.index());
    true
}

fn activate_position(state: &Rc<SearchState>, position: i32) {
    let Some(item) = usize::try_from(position)
        .ok()
        .and_then(|position| state.visible_results.borrow().get(position).cloned())
    else {
        return;
    };
    skip_focus_restore(state);
    hide(state);
    (state.activate)(item);
}

fn skip_focus_restore(state: &SearchState) {
    state.focus_restore.set(false);
}

fn hide(state: &SearchState) {
    hide_then(state, || {});
}

fn hide_then(state: &SearchState, after_dismiss: impl FnOnce() + 'static) {
    close_result_menu(state);
    state.generation.set(state.generation.get() + 1);
    record_interaction(state);
    state.search.borrow_mut().take();
    clear_results(state);
    state.truncated_hint.set_visible(false);
    state.indexing_spinner.stop();
    state.indexing_spinner.set_visible(false);
    if state.layer.has_css_class("dismissing") {
        return;
    }
    state.layer.add_css_class("dismissing");
    state.layer.set_sensitive(false);
    let layer = state.layer.clone();
    let dismiss = state.dismiss.clone();
    super::browser::animate_out(&state.layer, move || {
        layer.set_visible(false);
        layer.remove_css_class("dismissing");
        layer.set_sensitive(true);
        dismiss();
        after_dismiss();
    });
}

fn clear_results(state: &SearchState) {
    state.navigation_started.set(false);
    state.reconciling_results.set(true);
    state.visible_results.borrow_mut().clear();
    state.rendered_query.borrow_mut().clear();
    state.positions.borrow_mut().clear();
    state.requested_thumbnails.borrow_mut().clear();
    state.scroller.vadjustment().set_value(0.0);
    while let Some(child) = state.list.first_child() {
        super::thumbnail::cancel_thumbnails_in(&child);
        state.list.remove(&child);
    }
    state.reconciling_results.set(false);
}

/// The hint names why results are partial and, for limit stops, where to raise the limits.
fn show_coverage(hint: &gtk::Box, coverage: SearchCoverage) {
    let message = coverage.is_partial().then(|| coverage.message());
    crate::ui::accessibility::set_description(hint, message.as_deref());
    hint.set_tooltip_text(message.as_deref());
    hint.set_visible(message.is_some());
}

#[cfg(test)]
mod tests;
