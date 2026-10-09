// SPDX-License-Identifier: MIT

//! 10xer footer filter. It drives the focused pane's own filter field without
//! revealing the funnel, so that field's name rules, **Include subfolders**
//! scope, and stale-work cancellation stay the filter's rules.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{BrowserView, FilterQueryBinding, ViewState, columns::ColumnView};
use crate::{
    app::VisualKind,
    services::SearchItem,
    ui::{browser_modes::BrowserMode, inline_search::InlineSearch},
};

#[derive(Default)]
pub(super) struct FilterState {
    owner: RefCell<Weak<ViewState>>,
    handlers: RefCell<Vec<Rc<dyn Fn()>>>,
    /// A commit whose results were still loading; their arrival focuses the
    /// first result.
    focus_on_arrival: Cell<bool>,
    notify_scheduled: Cell<bool>,
    /// Whether a footer prompt that filters this browser, 10xer's **f** or **s**, has focus.
    footer_focus: RefCell<Option<Rc<dyn Fn() -> bool>>>,
}

impl FilterState {
    pub(super) fn set_owner(&self, state: &Rc<ViewState>) {
        self.owner.replace(Rc::downgrade(state));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct FilterStatus {
    pub query: String,
    pub search: bool,
    pub current: Option<PathBuf>,
    /// Whether results replace the listing. A typed query has not switched the
    /// view until its debounce fires, a search has nothing to count until its
    /// first batch, and a column on a non-local location narrows its own rows
    /// in place; the counts below describe none of these.
    pub displayed: bool,
    pub files: usize,
    pub folders: usize,
    pub visual: Option<VisualKind>,
}

/// Which part of the active pane's filter session holds keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum FilterFocus {
    /// The Ctrl+F field.
    Entry,
    /// The rows that replace the listing while the field has text: the results view in
    /// Icons and List, or a column's rows while it shows recursive hits or narrows in
    /// place.
    Results,
}

impl FilterStatus {
    pub(in crate::ui) fn total(&self) -> usize {
        self.files + self.folders
    }
}

/// The result `steps` rows from `current`: zero stays (on the first result
/// without a cursor) and `usize::MAX` jumps to an end.
pub(in crate::ui) fn results_step_target(
    current: Option<u32>,
    count: u32,
    direction: i32,
    steps: usize,
) -> Option<u32> {
    let last = count.checked_sub(1)?;
    if steps == 0 {
        return Some(current.map_or(0, |current| current.min(last)));
    }
    if steps == usize::MAX {
        return Some(if direction < 0 { 0 } else { last });
    }
    let steps = u32::try_from(steps).unwrap_or(u32::MAX);
    Some(match current {
        Some(current) if direction < 0 => current.saturating_sub(steps),
        Some(current) => current.saturating_add(steps).min(last),
        None if direction < 0 => last,
        None => 0,
    })
}

pub(in crate::ui) fn scroll_results_to(
    view: &gtk::Widget,
    position: u32,
    flags: gtk::ListScrollFlags,
) {
    if let Some(list) = view.downcast_ref::<gtk::ListView>() {
        list.scroll_to(position, flags, None);
    } else if let Some(grid) = view.downcast_ref::<gtk::GridView>() {
        grid.scroll_to(position, flags, None);
    }
}

pub(super) struct Hits {
    pub(super) selection: gtk::MultiSelection,
    pub(super) view: gtk::Widget,
    pub(super) cursor: Option<u32>,
}

impl Hits {
    pub(super) fn count(&self) -> u32 {
        self.selection.n_items()
    }
}

pub(super) enum Target {
    Column(Box<ColumnView>),
    Pane {
        entry: gtk::Entry,
        button: gtk::ToggleButton,
        search: InlineSearch,
    },
}

impl Target {
    pub(super) fn entry(&self) -> &gtk::Entry {
        match self {
            Self::Column(column) => &column.filter_entry,
            Self::Pane { entry, .. } => entry,
        }
    }

    fn with_binding<T>(&self, apply: impl FnOnce(&FilterQueryBinding) -> T) -> Option<T> {
        match self {
            Self::Column(column) => column.with_query_binding(apply),
            Self::Pane { search, .. } => search.with_query_binding(apply),
        }
    }

    /// List and Icons panes have none without a local folder.
    pub(super) fn has_query_binding(&self) -> bool {
        self.with_binding(|_| ()).is_some()
    }

    pub(super) fn forced_recursive(&self) -> bool {
        self.with_binding(FilterQueryBinding::forced_recursive)
            .unwrap_or(false)
    }

    /// Applies text still waiting on its debounce, dropping the previous
    /// query's rows so focus lands on rows that belong to the new one.
    pub(super) fn settle(&self) {
        if self.with_binding(FilterQueryBinding::settle).is_none() {
            self.flush();
        }
    }

    pub(super) fn searching(&self) -> bool {
        self.forced_recursive() && !self.entry().text().trim().is_empty()
    }

    /// Shows `query` in the field's own scope, or in its subfolders when
    /// `forced`. Returns `false` when this listing cannot search subfolders.
    pub(super) fn apply(&self, query: &str, forced: bool) -> bool {
        let rescoped = self.with_binding(|binding| binding.force_recursive(forced));
        if forced && rescoped.is_none() {
            return false;
        }
        if self.entry().text() != query {
            set_filter_text(self, query);
        } else if rescoped == Some(true) {
            self.with_binding(FilterQueryBinding::requery);
        }
        true
    }

    fn button(&self) -> &gtk::ToggleButton {
        match self {
            Self::Column(column) => &column.filter_button,
            Self::Pane { button, .. } => button,
        }
    }

    pub(super) fn flush(&self) {
        match self {
            Self::Column(column) => column.flush_filter_query(),
            Self::Pane { search, .. } => search.flush_query(),
        }
    }

    pub(super) fn results_view(&self) -> Option<gtk::Widget> {
        match self {
            Self::Column(column) => column
                .recursive_search_active
                .get()
                .then(|| column.list.clone().upcast()),
            Self::Pane { search, .. } => search.results_view(),
        }
    }

    fn awaiting_results(&self) -> bool {
        match self {
            Self::Column(column) => column.search_session.awaiting_results(),
            Self::Pane { search, .. } => search.awaiting_results(),
        }
    }

    pub(super) fn hits(&self) -> Option<Hits> {
        match self {
            Self::Column(column) => column.recursive_search_active.get().then(|| Hits {
                selection: column.selection.clone(),
                view: column.list.clone().upcast(),
                cursor: column_cursor(column),
            }),
            Self::Pane { search, .. } => search.hits().map(|(selection, view, cursor)| Hits {
                selection,
                view,
                cursor,
            }),
        }
    }

    pub(super) fn results(&self) -> Option<Vec<SearchItem>> {
        match self {
            Self::Column(column) => column
                .recursive_search_active
                .get()
                .then(|| column.search_results.borrow().clone()),
            Self::Pane { search, .. } => search.results(),
        }
    }

    pub(super) fn current_result(&self) -> Option<SearchItem> {
        match self {
            Self::Column(column) => {
                if !column.recursive_search_active.get() {
                    return None;
                }
                let position = column_cursor(column)?;
                column
                    .search_results
                    .borrow()
                    .get(position as usize)
                    .cloned()
            }
            Self::Pane { search, .. } => search.current_result(),
        }
    }

    pub(super) fn step(&self, direction: i32, steps: usize, take_focus: bool) -> bool {
        match self {
            Self::Column(column) => step_column_results(column, direction, steps, take_focus),
            Self::Pane { search, .. } => search.step(direction, steps, take_focus),
        }
    }

    fn invert(&self) -> bool {
        match self {
            Self::Column(column) => {
                if !column.recursive_search_active.get() {
                    return false;
                }
                let count = column.selection.n_items();
                if count == 0 {
                    return false;
                }
                let inverted = gtk::Bitset::new_range(0, count);
                inverted.subtract(&column.selection.selection());
                column
                    .selection
                    .set_selection(&inverted, &gtk::Bitset::new_range(0, count));
                true
            }
            Self::Pane { search, .. } => search.invert_selection(),
        }
    }
}

/// Only a revealed funnel counts: a 10xer footer filter leaves it closed and has its
/// own Escape order.
pub(super) fn column_filter_focus(
    column: &ColumnView,
    focused: &gtk::Widget,
) -> Option<FilterFocus> {
    let within = |widget: &gtk::Widget| focused == widget || focused.is_ancestor(widget);
    if within(column.filter_entry.upcast_ref()) {
        return Some(FilterFocus::Entry);
    }
    (within(column.list.upcast_ref())
        && column.filter_button.is_active()
        && (column.recursive_search_active.get() || column.map.has_query()))
    .then_some(FilterFocus::Results)
}

pub(super) fn column_cursor(column: &ColumnView) -> Option<u32> {
    let focused = column.list.root().and_then(|root| root.focus());
    focused
        .and_then(|focused| {
            column.bound_rows.borrow().iter().find_map(|bound| {
                let row = bound.row.upgrade()?;
                let row: &gtk::Widget = row.upcast_ref();
                // GTK focuses the list item widget that holds the row.
                (focused == *row
                    || focused.is_ancestor(row)
                    || row.parent().as_ref() == Some(&focused))
                .then(|| bound.item.upgrade().map(|item| item.position()))
                .flatten()
            })
        })
        .or_else(|| {
            let selected = column.selection.selection();
            (!selected.is_empty()).then(|| selected.maximum())
        })
}

/// Outside a fill the selection is the cursor; a row GTK kept focused may
/// belong to a previous query.
pub(in crate::ui) fn selected_cursor(selection: &gtk::MultiSelection) -> Option<u32> {
    let selected = selection.selection();
    (!selected.is_empty()).then(|| selected.maximum())
}

fn step_column_results(
    column: &ColumnView,
    direction: i32,
    steps: usize,
    take_focus: bool,
) -> bool {
    if !column.recursive_search_active.get() {
        return false;
    }
    let current = if steps == 0 {
        selected_cursor(&column.selection)
    } else {
        column_cursor(column)
    };
    let Some(target) = results_step_target(current, column.selection.n_items(), direction, steps)
    else {
        return true;
    };
    column.selection.select_item(target, true);
    let flags = if take_focus {
        column.list.grab_focus();
        gtk::ListScrollFlags::FOCUS
    } else {
        gtk::ListScrollFlags::NONE
    };
    column.list.scroll_to(target, flags, None);
    true
}

fn filter_status(target: &Target, root: Option<&Path>) -> Option<FilterStatus> {
    let query = target.entry().text();
    if query.trim().is_empty() {
        return None;
    }
    let results = target.results().unwrap_or_default();
    let folders = results.iter().filter(|item| item.is_directory).count();
    let search = target.searching();
    let current = search
        .then(|| target.current_result())
        .flatten()
        .and_then(|item| Some(item.path.strip_prefix(root?).ok()?.to_path_buf()));
    Some(FilterStatus {
        query: query.to_string(),
        search,
        current,
        displayed: target.results_view().is_some()
            && !(results.is_empty() && target.awaiting_results()),
        files: results.len() - folders,
        folders,
        visual: None,
    })
}

pub(super) fn filter_shows_query(target: &Target) -> bool {
    !target.entry().text().trim().is_empty()
}

pub(super) fn set_filter_text(target: &Target, query: &str) {
    if query.is_empty() {
        // Closing a funnel that 10xer hides clears nothing; the text is cleared below.
        target.button().set_active(false);
    }
    if target.entry().text() != query {
        target.entry().set_text(query);
    }
}

impl ViewState {
    pub(super) fn filter_target(&self) -> Option<Target> {
        self.try_filter_target().flatten()
    }

    /// `None` while a rebuild holds the views, which browser events can
    /// interrupt.
    fn try_filter_target(&self) -> Option<Option<Target>> {
        if self.mode.get() == BrowserMode::Columns {
            let columns = self.columns.try_borrow().ok()?;
            return Some(self.filter_depth().and_then(|depth| {
                columns
                    .get(depth)
                    .cloned()
                    .map(|column| Target::Column(Box::new(column)))
            }));
        }
        let panes = self.mode_views.try_borrow().ok()?;
        Some(
            panes
                .active_filter()
                .map(|(entry, button, search)| Target::Pane {
                    entry,
                    button,
                    search,
                }),
        )
    }

    /// `None` when the listing, chrome or something outside the panes has focus.
    pub(super) fn filter_focus(&self) -> Option<FilterFocus> {
        if self.mode.get() != BrowserMode::Columns {
            return self.mode_views.borrow().filter_focus();
        }
        let focused = self.overlay.root()?.focus()?;
        self.columns
            .borrow()
            .iter()
            .find_map(|column| column_filter_focus(column, &focused))
    }

    pub(in crate::ui) fn footer_filter_has_focus(&self) -> bool {
        self.listing_filter
            .footer_focus
            .borrow()
            .as_ref()
            .is_some_and(|has_focus| has_focus())
    }

    /// An outside change never pulls focus out of a focused filter: the pane's field or
    /// results, or a footer prompt that filters the listing.
    pub(super) fn outside_change_keeps_focus(&self) -> bool {
        self.browser.focus_follows_external_change()
            && (self.filter_focus().is_some() || self.footer_filter_has_focus())
    }

    fn filter_depth(&self) -> Option<usize> {
        if self.mode.get() == BrowserMode::Columns {
            self.focused_column_depth()
                .or_else(|| self.browser.active_depth())
        } else {
            self.browser.active_depth()
        }
    }
}

impl BrowserView {
    pub(super) fn filter_target(&self) -> Option<Target> {
        self.state.filter_target()
    }

    pub(in crate::ui) fn filter_focus(&self) -> Option<FilterFocus> {
        self.state.filter_focus()
    }

    pub(in crate::ui) fn listing_filter(&self) -> Option<String> {
        let target = self.filter_target()?;
        let text = if target.searching() {
            self.saved_listing_filter(&target)
        } else {
            target.entry().text().to_string()
        };
        (!text.trim().is_empty()).then_some(text)
    }

    /// Filters the focused listing by `query` (an empty query clears it) and
    /// leaves the funnel closed.
    pub(in crate::ui) fn set_listing_filter(&self, query: &str) {
        let Some(target) = self.filter_target() else {
            return;
        };
        self.clear_other_column_filters(&target);
        // A filter replaces a showing search rather than narrowing its hits.
        self.end_listing_search_for_filter();
        target.apply(query, false);
        self.state.notify_filter_results_changed();
    }

    pub(super) fn clear_other_column_filters(&self, target: &Target) {
        let Target::Column(column) = target else {
            return;
        };
        let others: Vec<_> = self
            .state
            .columns
            .borrow()
            .iter()
            .filter(|other| other.filter_entry != column.filter_entry)
            .map(|other| Target::Column(Box::new(other.clone())))
            .collect();
        for other in others
            .iter()
            .filter(|other| !other.entry().text().is_empty())
        {
            set_filter_text(other, "");
        }
    }

    /// Applies `query` at once and returns keyboard focus to the filtered
    /// listing without opening an item.
    pub(in crate::ui) fn commit_listing_filter(&self, query: &str) {
        self.set_listing_filter(query);
        let Some(target) = self.filter_target() else {
            return;
        };
        target.flush();
        self.focus_filter_results(&target);
    }

    pub(in crate::ui) fn focus_visible_results(&self) -> bool {
        let Some(target) = self
            .filter_target()
            .filter(|target| target.results_view().is_some())
        else {
            return false;
        };
        self.focus_filter_results(&target);
        true
    }

    /// The rows showing may still be a previous query's, so arriving rows
    /// take focus again until it moves elsewhere.
    pub(super) fn focus_filter_results(&self, target: &Target) {
        self.keyboard_navigation();
        let Some(view) = target.results_view() else {
            self.state.browser.focus_active();
            return;
        };
        self.state.listing_filter.focus_on_arrival.set(true);
        if target.selection_is_empty() {
            view.grab_focus();
        } else if !self.state.focus_filled_results() {
            target.step(1, 0, true);
        }
    }

    /// Returns keyboard focus to the results replacing the listing, on the
    /// row they last focused, without rewriting their selection. Returns
    /// `false` without results.
    pub(in crate::ui) fn focus_results_cursor(&self) -> bool {
        let Some(view) = self
            .filter_target()
            .and_then(|target| target.results_view())
        else {
            return false;
        };
        self.keyboard_navigation();
        view.grab_focus();
        true
    }

    pub(in crate::ui) fn clear_listing_filter(&self) -> bool {
        let Some(target) = self.filter_target() else {
            return false;
        };
        if target.entry().text().is_empty() {
            return false;
        }
        self.state.listing_filter.focus_on_arrival.set(false);
        set_filter_text(&target, "");
        target.flush();
        self.state.notify_filter_results_changed();
        self.keyboard_navigation();
        self.state.browser.focus_active();
        true
    }

    pub(in crate::ui) fn release_forced_recursion(&self) {
        let columns: Vec<_> = self.state.columns.borrow().clone();
        for column in columns {
            column.with_query_binding(FilterQueryBinding::release_forced_recursion);
        }
        let panes = self.state.mode_views.borrow().pane_searches();
        for search in panes {
            search.with_query_binding(FilterQueryBinding::release_forced_recursion);
        }
    }

    /// Clears the filters 10xer left behind a closed funnel in every pane.
    pub(in crate::ui) fn clear_hidden_filters(&self) {
        self.state.listing_filter.focus_on_arrival.set(false);
        self.state.forget_result_fill();
        let columns: Vec<_> = self
            .state
            .columns
            .borrow()
            .iter()
            .filter(|column| !column.filter_button.is_active())
            .map(|column| column.filter_entry.clone())
            .collect();
        let panes = self.state.mode_views.borrow().hidden_filter_entries();
        for entry in columns.into_iter().chain(panes) {
            entry.set_text("");
        }
    }

    /// The focused listing's filter; `None` while a rebuild holds the views,
    /// so callers retry rather than report a stale listing.
    pub(in crate::ui) fn filter_status(&self) -> Option<Option<FilterStatus>> {
        let target = self.state.try_filter_target()?;
        let root = self
            .state
            .filter_depth()
            .and_then(|depth| self.state.browser.location_at(depth));
        let root = root.as_ref().and_then(|location| location.native_path());
        let status = target.and_then(|target| filter_status(&target, root));
        Some(status.map(|status| FilterStatus {
            visual: self.state.result_visual_kind(),
            ..status
        }))
    }

    pub(in crate::ui) fn results_replace_listing(&self) -> bool {
        self.filter_target()
            .is_some_and(|target| target.results_view().is_some())
    }

    pub(super) fn place_found_result(&self, position: u32, take_focus: bool) {
        let Some(hits) = self.filter_target().and_then(|target| target.hits()) else {
            return;
        };
        self.keyboard_navigation();
        self.keep_result_fill(|| {
            hits.selection.select_item(position, true);
            let flags = if take_focus {
                hits.view.grab_focus();
                gtk::ListScrollFlags::FOCUS
            } else {
                gtk::ListScrollFlags::NONE
            };
            scroll_results_to(&hits.view, position, flags);
        });
    }

    /// Moves among filter results instead of the hidden directory cursor.
    /// Returns whether results are showing.
    pub(in crate::ui) fn step_filter_results(
        &self,
        direction: i32,
        steps: usize,
        take_focus: bool,
    ) -> bool {
        if take_focus && self.step_filled_results(direction, steps) {
            return true;
        }
        self.filter_target()
            .is_some_and(|target| target.step(direction, steps, take_focus))
    }

    /// Inverts the selection among filter results. Returns whether results are
    /// showing.
    /// Recursive **s** hits claim the key without inverting.
    pub(in crate::ui) fn invert_filter_results(&self) -> bool {
        self.filter_target().is_some_and(|target| {
            target.results_view().is_some() && {
                if !target.searching() && target.invert() {
                    self.state.remember_result_selection(&target);
                    // A late arrival would collapse the inverted selection to its cursor.
                    self.state.listing_filter.focus_on_arrival.set(false);
                }
                true
            }
        })
    }

    #[cfg(test)]
    pub(in crate::ui) fn filter_result_names(&self) -> Vec<String> {
        self.filter_target()
            .and_then(|target| target.results())
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.name)
            .collect()
    }

    #[cfg(test)]
    pub(in crate::ui) fn extend_result_selection(&self, position: u32) -> bool {
        self.filter_target()
            .and_then(|target| target.hits())
            .is_some_and(|hits| hits.selection.select_item(position, false))
    }

    pub(in crate::ui) fn set_footer_filter_focus(&self, has_focus: Rc<dyn Fn() -> bool>) {
        self.state
            .listing_filter
            .footer_focus
            .replace(Some(has_focus));
    }

    pub(in crate::ui) fn connect_filter_results_changed(&self, handler: Rc<dyn Fn()>) {
        self.state
            .listing_filter
            .handlers
            .borrow_mut()
            .push(handler);
    }
}

impl Target {
    pub(super) fn selection_is_empty(&self) -> bool {
        match self {
            Self::Column(column) => column.selection.n_items() == 0,
            Self::Pane { search, .. } => search.results().is_none_or(|results| results.is_empty()),
        }
    }
}

impl ViewState {
    /// Results can change while a pane is being built with its views borrowed,
    /// so observers run once on idle for any burst of changes.
    pub(in crate::ui) fn notify_filter_results_changed(&self) {
        if self.listing_filter.notify_scheduled.replace(true) {
            return;
        }
        let owner = self.listing_filter.owner.borrow().clone();
        glib::idle_add_local_once(move || {
            let Some(state) = owner.upgrade() else {
                return;
            };
            state.listing_filter.notify_scheduled.set(false);
            if state.listing_filter.focus_on_arrival.get() {
                state.focus_arrived_results();
            }
            let handlers = state.listing_filter.handlers.borrow().clone();
            for handler in handlers {
                handler();
            }
        });
    }

    fn focus_arrived_results(&self) {
        let Some(target) = self.filter_target() else {
            return;
        };
        let Some(results) = target.results_view() else {
            self.listing_filter.focus_on_arrival.set(false);
            return;
        };
        // Replacing the focused row leaves focus on a container of the results,
        // or on the detached row itself.
        let focus = self.overlay.root().and_then(|root| root.focus());
        let still_waiting = focus.as_ref().is_none_or(|focus| {
            focus.root().is_none()
                || focus == &results
                || focus.is_ancestor(&results)
                || results.is_ancestor(focus)
        });
        if !still_waiting {
            self.listing_filter.focus_on_arrival.set(false);
            return;
        }
        if !target.selection_is_empty() && !self.focus_filled_results() {
            target.step(1, 0, true);
        }
    }
}
