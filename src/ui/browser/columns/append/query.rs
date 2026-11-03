// SPDX-License-Identifier: MIT

//! A column's filter query: in-place filtering and the recursive search behind it.

use super::super::{ColumnHits, search::ResultLists};
use super::header::ColumnHeader;
use super::models::ColumnModels;
use crate::app::Browser;
use crate::services::fold_for_search;
use crate::ui::browser::ViewState;
use crate::ui::browser::collection::{
    FilterQueryBinding, apply_filter_query, bind_filter_query, deactivate_recursive_search,
};
use crate::ui::browser::find::{HitRanges, search_hit_ranges};
use crate::ui::enter_target::EnterTarget;
use crate::ui::entry_list_model::EntryListModel;
use crate::ui::search_session::{SearchBatch, SearchInput, SearchScope, SearchSession};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

/// The search-hit list a query fills, shared with each search batch.
#[derive(Clone)]
struct SearchHits {
    lists: ResultLists,
    hits: Rc<RefCell<ColumnHits>>,
    enter: EnterTarget,
}

/// Delivers one recursive search's batches into the column at `depth`.
struct SearchBatches {
    hits: SearchHits,
    browser: Weak<Browser>,
    state: Weak<ViewState>,
    depth: usize,
    root: PathBuf,
    scope: SearchScope,
}

impl SearchBatches {
    fn deliver(&self, batch: SearchBatch) {
        let (Some(browser), Some(state)) = (self.browser.upgrade(), self.state.upgrade()) else {
            return;
        };
        let session = state
            .columns
            .borrow()
            .get(self.depth)
            .map(|column| column.search_session.clone());
        let Some(session) = session else {
            return;
        };
        let items = crate::ui::inline_search::eligible_results(
            &browser,
            &session,
            &batch.query,
            batch.items,
            batch.has_more,
            batch.candidates,
        );
        self.hits.hits.replace(ColumnHits {
            ranges: if self.scope.fuzzy() {
                search_hit_ranges(&batch.query, &self.root, &items)
            } else {
                HitRanges::new()
            },
            recursive: self.scope.recursive(),
        });
        let keep_selection = !self.hits.enter.show_query(&batch.query);
        let update = self.hits.lists.update(items, keep_selection);
        if let Some(position) = update.renamed {
            state.focus_renamed_column_result(self.depth, position);
        }
        if update.selection_changed {
            state.notify_search_selection_changed();
        }
        state.refresh_name_highlights();
        state.notify_filter_results_changed();
    }
}

/// One non-empty query typed into the filter field.
struct SearchRequest<'a> {
    text: &'a str,
    /// `text` trimmed.
    query: String,
    scope: SearchScope,
    restart: bool,
}

/// What the filter field's query handler reads and updates.
struct ColumnQuery {
    state: Weak<ViewState>,
    depth: usize,
    filtered_model: gtk::FilterListModel,
    model: EntryListModel,
    filter: gtk::CustomFilter,
    filter_query: Rc<RefCell<String>>,
    search_active: Rc<Cell<bool>>,
    session: SearchSession,
    hits: SearchHits,
}

impl ColumnQuery {
    fn on_query(&self, text: String, scope: SearchScope, restart: bool) {
        if restart {
            self.restart();
        }
        let query = text.trim().to_string();
        if query.is_empty() {
            self.clear(&text);
            return;
        }
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let Some(path) = state
            .browser
            .location_at(self.depth)
            .and_then(|loc| loc.native_path().map(Path::to_path_buf))
        else {
            self.filter_in_place(&state, &text);
            return;
        };
        let request = SearchRequest {
            text: &text,
            query,
            scope,
            restart,
        };
        self.search_recursively(&state, path, &request);
    }

    fn restart(&self) {
        self.hits.enter.restart();
        self.session.cancel();
        let changed = self.hits.lists.update(Vec::new(), false).selection_changed;
        if changed && let Some(state) = self.state.upgrade() {
            state.notify_search_selection_changed();
        }
    }

    fn apply_listing_filter(&self, text: &str) {
        apply_filter_query(
            &self.filtered_model,
            &self.filter,
            &self.filter_query,
            fold_for_search(text),
        );
    }

    fn deactivate_search(&self) {
        deactivate_recursive_search(
            &self.search_active,
            &self.hits.lists.results,
            &self.hits.lists.model,
            &self.filtered_model,
            &self.model,
        );
    }

    fn clear(&self, text: &str) {
        self.hits.lists.renamed.clear();
        self.hits.enter.clear();
        self.session.cancel();
        // Keep the hidden-file filter installed while swapping back to the directory
        // model; GTK's synchronous model notifications otherwise leave a stale row.
        self.apply_listing_filter(text);
        self.deactivate_search();
        if let Some(state) = self.state.upgrade() {
            state.resync_column_selection(self.depth);
            state.refresh_name_highlights();
            state.notify_filter_results_changed();
        }
    }

    /// A folder without a native path filters its loaded listing instead of searching.
    fn filter_in_place(&self, state: &ViewState, text: &str) {
        self.session.cancel();
        self.deactivate_search();
        // The in-place selection is the browser's own; a new query
        // only stops it from counting as a pick.
        self.hits.enter.show_query(text);
        self.apply_listing_filter(text);
        state.refresh_name_highlights();
        state.notify_filter_results_changed();
    }

    fn search_recursively(&self, state: &ViewState, path: PathBuf, request: &SearchRequest<'_>) {
        *self.filter_query.borrow_mut() = fold_for_search(request.text);
        self.search_active.set(true);
        if !self.session.is_active() {
            self.filtered_model.set_filter(None::<&gtk::CustomFilter>);
            self.filtered_model.set_model(Some(&self.hits.lists.model));
        }
        let show_hidden = state
            .browser
            .column_preferences(self.depth)
            .unwrap_or_else(|| state.browser.preferences())
            .show_hidden;
        let batches = SearchBatches {
            hits: self.hits.clone(),
            browser: Rc::downgrade(&state.browser),
            state: self.state.clone(),
            depth: self.depth,
            root: path.clone(),
            scope: request.scope,
        };
        self.session.update(
            SearchInput {
                root: path,
                show_hidden,
                scope: request.scope,
                refused: Default::default(),
                within: state.browser.confined_to(),
            },
            &request.query,
            request.restart,
            Rc::new(move |batch| batches.deliver(batch)),
        );
    }
}

impl ViewState {
    pub(super) fn bind_column_query(
        self: &Rc<Self>,
        depth: usize,
        header: &ColumnHeader,
        models: &ColumnModels,
    ) -> FilterQueryBinding {
        let query = ColumnQuery {
            state: Rc::downgrade(self),
            depth,
            filtered_model: models.filtered_model.clone(),
            model: models.model.clone(),
            filter: models.filter.clone(),
            filter_query: models.filter_query.clone(),
            search_active: models.recursive_search_active.clone(),
            session: models.search_session.clone(),
            hits: SearchHits {
                lists: ResultLists {
                    model: models.search_model.clone(),
                    results: models.search_results.clone(),
                    selection: models.selection.clone(),
                    syncing: models.syncing_selection.clone(),
                    renamed: models.renamed_result.clone(),
                },
                hits: models.hits.clone(),
                enter: header.enter_target.clone(),
            },
        };
        bind_filter_query(
            &header.filter_entry,
            &models.search_session,
            move |text, scope, restart| query.on_query(text, scope, restart),
        )
    }
}
