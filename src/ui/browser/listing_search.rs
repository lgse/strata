// SPDX-License-Identifier: MIT

//! 10xer recursive **s** search. It borrows the focused listing's filter field
//! with its scope forced to include subfolders, so the filter's name rules, the
//! 100-hit cap, and stale-work cancellation stay the search's rules. The saved
//! **Include subfolders** preference is never written, and dismissal puts back
//! the **f** query the search replaced.

use std::cell::{Cell, RefCell};

use gtk::{glib, prelude::*};

use super::{BrowserView, listing_filter::Target};
use crate::{model::Location, ui::browser_modes::BrowserMode};

#[derive(Default)]
pub(super) struct SearchState {
    borrowed: RefCell<Option<(glib::WeakRef<gtk::Entry>, String)>>,
    /// A hit's Miller column is opening beside the hits, not replacing them.
    opening_hit_column: Cell<bool>,
}

impl SearchState {
    fn saved_filter(&self, target: &Target) -> String {
        self.borrowed
            .borrow()
            .as_ref()
            .filter(|(entry, _)| entry.upgrade().as_ref() == Some(target.entry()))
            .map(|(_, saved)| saved.clone())
            .unwrap_or_default()
    }
}

impl BrowserView {
    fn search_target(&self) -> Option<Target> {
        self.filter_target().filter(Target::searching)
    }

    /// A listing without a local folder filters its rows in place.
    fn supports_listing_search(&self, target: &Target) -> bool {
        if !target.has_query_binding() {
            return false;
        }
        if self.state.mode.get() != BrowserMode::Columns {
            return true;
        }
        self.state
            .focused_column_depth()
            .or_else(|| self.state.browser.active_depth())
            .and_then(|depth| self.state.browser.location_at(depth))
            .is_some_and(|location| location.native_path().is_some())
    }

    pub(super) fn saved_listing_filter(&self, target: &Target) -> String {
        self.state.listing_search.saved_filter(target)
    }

    pub(super) fn end_listing_search_for_filter(&self) {
        self.state.listing_search.borrowed.take();
    }

    pub(in crate::ui) fn listing_search_active(&self) -> bool {
        self.search_target().is_some()
    }

    /// Starts an **s** search in the focused listing, remembering its **f**
    /// query. Returns the text to pre-fill: the current query when a search is
    /// already showing, or `None` when this listing cannot search subfolders.
    pub(in crate::ui) fn begin_listing_search(&self) -> Option<String> {
        let target = self.filter_target()?;
        if target.searching() {
            return Some(target.entry().text().to_string());
        }
        if !self.supports_listing_search(&target) {
            return None;
        }
        self.state.listing_search.borrowed.replace(Some((
            target.entry().downgrade(),
            target.entry().text().to_string(),
        )));
        Some(String::new())
    }

    /// Shows the hits for `query` below the focused folder. An empty query
    /// shows the listing as the search found it.
    pub(in crate::ui) fn set_listing_search(&self, query: &str) {
        let Some(target) = self.filter_target() else {
            return;
        };
        if query.trim().is_empty() {
            let saved = self.state.listing_search.saved_filter(&target);
            target.apply(&saved, false);
        } else if self.supports_listing_search(&target) {
            // The hidden directory's range would otherwise keep the footer
            // mark and the first Esc. Its fill stays, and the prompt keeps focus.
            self.state.browser.take_visual();
            self.clear_other_column_filters(&target);
            target.apply(query, true);
        }
        self.state.notify_filter_results_changed();
    }

    /// Applies `query` at once and returns keyboard focus to its hits without
    /// opening one. An empty query dismisses the search.
    pub(in crate::ui) fn commit_listing_search(&self, query: &str) {
        if query.trim().is_empty() {
            if !self.dismiss_listing_search() {
                self.state.browser.focus_active();
            }
            return;
        }
        self.set_listing_search(query);
        let Some(target) = self.search_target() else {
            self.state.browser.focus_active();
            return;
        };
        target.settle();
        self.focus_filter_results(&target);
    }

    /// Puts back the **f** filter the search replaced, or the directory.
    pub(in crate::ui) fn dismiss_listing_search(&self) -> bool {
        let borrowed = self.state.listing_search.borrowed.borrow().is_some();
        let Some(target) = self.filter_target() else {
            self.state.listing_search.borrowed.take();
            return false;
        };
        let searching = target.searching();
        if !searching && !borrowed {
            return false;
        }
        let saved = self.state.listing_search.saved_filter(&target);
        self.state.listing_search.borrowed.take();
        // An emptied prompt already put the filter back; it may still be settling.
        if searching {
            target.apply(&saved, false);
        }
        target.settle();
        self.state.notify_filter_results_changed();
        self.focus_filter_results(&target);
        true
    }

    /// Opens the folder holding the hit under the cursor with that item
    /// selected, ending the search. Returns `false` without a hit.
    pub(in crate::ui) fn reveal_listing_search_hit(&self) -> bool {
        let Some(hit) = self
            .search_target()
            .and_then(|target| target.current_result())
        else {
            return false;
        };
        self.forget_listing_search();
        self.keyboard_navigation();
        self.reveal_location(Location::local(hit.path));
        true
    }

    pub(in crate::ui) fn forget_listing_search(&self) {
        self.state.forget_listing_search();
    }

    /// Opens a directory hit in the column after `depth` and leaves the
    /// search showing.
    pub(in crate::ui) fn open_hit_column(&self, depth: usize, location: Location) {
        let search = &self.state.listing_search;
        let nested = search.opening_hit_column.replace(true);
        self.state.browser.show_child(depth, location);
        search.opening_hit_column.set(nested);
    }
}

impl super::ViewState {
    /// The column whose filter field the search borrowed.
    pub(super) fn listing_search_depth(&self) -> Option<usize> {
        let entry = self
            .listing_search
            .borrowed
            .borrow()
            .as_ref()?
            .0
            .upgrade()?;
        if self.mode.get() != BrowserMode::Columns {
            return self.browser.active_depth();
        }
        self.columns
            .try_borrow()
            .ok()?
            .iter()
            .position(|column| column.filter_entry == entry)
    }

    /// Drops the search without restoring anything, for navigation and
    /// leaving the mode: its hits never outlive the folder they came from.
    pub(super) fn forget_listing_search(&self) {
        if self.listing_search.opening_hit_column.get() {
            return;
        }
        self.listing_search.borrowed.take();
        if let Some(target) = self.filter_target().filter(Target::forced_recursive) {
            target.apply("", false);
        }
    }

    pub(super) fn carry_listing_search(&self, searching: bool) {
        if !searching {
            return;
        }
        let Some(target) = self.filter_target() else {
            return;
        };
        let saved = self
            .listing_search
            .borrowed
            .borrow()
            .as_ref()
            .map(|(_, saved)| saved.clone())
            .unwrap_or_default();
        self.listing_search
            .borrowed
            .replace(Some((target.entry().downgrade(), saved)));
        if !target.entry().text().trim().is_empty() {
            target.apply(&target.entry().text(), true);
        }
    }

    pub(super) fn listing_search_showing(&self) -> bool {
        self.filter_target()
            .is_some_and(|target| target.searching())
    }
}
