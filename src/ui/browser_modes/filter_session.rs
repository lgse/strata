// SPDX-License-Identifier: MIT

//! The active pane's filter session as the window sees it: its field, toggle, and
//! results.

use gtk::prelude::*;

use super::{
    ModeViews, filter::pane_filter_focus, focus::widget_has_focus, pane::focus_pane_surface,
};
use crate::{
    model::FileEntry,
    ui::{
        browser::{self, ActivePaneFilter, FilterFocus},
        inline_search::InlineSearch,
    },
};

impl ModeViews {
    pub(in crate::ui) fn settle_in_place_filter(&self) {
        let Some(pane) = self.single_pane() else {
            return;
        };
        let Some(entry) = pane.filter_entry.as_ref() else {
            return;
        };
        browser::notify_filter_query(&pane.filter, &pane.filter_query, entry.text().to_string());
    }

    pub(in crate::ui) fn clear_filter(&self, depth: usize) {
        for pane in self
            .all_panes()
            .into_iter()
            .filter(|pane| pane.depth == depth)
        {
            if let Some(field) = pane.filter_entry.as_ref() {
                field.set_text("");
            }
        }
    }

    pub(in crate::ui) fn filter_focus(&self) -> Option<FilterFocus> {
        let focused = self.stack.root().and_then(|root| root.focus());
        // Previous-mode panes stay in the tree unmapped; a just-revealed field
        // can have focus before GTK maps it.
        self.visible_panes()
            .into_iter()
            .find_map(|pane| pane_filter_focus(pane, focused.as_ref()))
    }

    pub fn selected_search_result(&self) -> Option<FileEntry> {
        self.single_pane()?.search.selected_entry()
    }

    pub fn focus_search_results(&self) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.focus_current())
    }

    pub fn focus_search_result(&self, path: &std::path::Path) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.focus_result(path))
    }

    pub fn select_search_result(&self, path: &std::path::Path) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.select_result(path))
    }

    pub fn select_all_search_results(&self) -> bool {
        self.single_pane()
            .is_some_and(|pane| pane.search.select_all())
    }

    pub fn selected_search_results(&self) -> Option<Vec<FileEntry>> {
        self.single_pane()?.search.selected_entries()
    }

    pub(in crate::ui) fn filter_active(&self) -> bool {
        self.selected_search_results().is_some()
            || self
                .visible_panes()
                .into_iter()
                .filter_map(|pane| pane.filter_entry.as_ref())
                .any(|entry| !entry.text().trim().is_empty())
    }

    pub fn empty_filter_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.visible_panes()
            .into_iter()
            .filter_map(|pane| pane.filter_entry.as_ref())
            .any(|entry| entry.text().is_empty() && widget_has_focus(entry, focused.as_ref()))
    }

    pub fn show_filter_with_query(&self, query: Option<&str>) -> bool {
        let Some(pane) = self.single_pane() else {
            return false;
        };
        let (Some(entry), Some(button)) = (pane.filter_entry.as_ref(), pane.filter_button.as_ref())
        else {
            return false;
        };
        button.set_active(true);
        browser::focus_filter_entry(entry, query);
        true
    }

    /// The active pane's filter field, its funnel toggle, and the results that
    /// replace its listing while the field has text.
    pub(in crate::ui) fn active_filter(
        &self,
    ) -> Option<(gtk::Entry, gtk::ToggleButton, InlineSearch)> {
        self.filter_at(self.browser.active_depth()?)
    }

    pub(in crate::ui) fn filter_at(
        &self,
        depth: usize,
    ) -> Option<(gtk::Entry, gtk::ToggleButton, InlineSearch)> {
        let pane = self.panes_at(depth).into_iter().next()?;
        Some((
            pane.filter_entry.clone()?,
            pane.filter_button.clone()?,
            pane.search.clone(),
        ))
    }

    pub(in crate::ui) fn pane_searches(&self) -> Vec<InlineSearch> {
        self.all_panes()
            .into_iter()
            .map(|pane| pane.search.clone())
            .collect()
    }

    pub(in crate::ui) fn hidden_filter_entries(&self) -> Vec<gtk::Entry> {
        self.all_panes()
            .into_iter()
            .filter(|pane| {
                pane.filter_button
                    .as_ref()
                    .is_some_and(|button| !button.is_active())
            })
            .filter_map(|pane| pane.filter_entry.clone())
            .filter(|entry| !entry.text().is_empty())
            .collect()
    }

    pub(crate) fn capture_active_filter(&self) -> ActivePaneFilter {
        let Some(depth) = self.browser.active_depth() else {
            return ActivePaneFilter::default();
        };
        let Some(pane) = self.panes_at(depth).into_iter().next() else {
            return ActivePaneFilter::default();
        };
        ActivePaneFilter {
            query: pane
                .filter_entry
                .as_ref()
                .map(|entry| entry.text().to_string())
                .unwrap_or_default(),
            revealed: pane
                .filter_button
                .as_ref()
                .is_some_and(|button| button.is_active()),
        }
    }

    pub(crate) fn restore_active_filter(&self, filter: &ActivePaneFilter) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let Some(pane) = self.panes_at(depth).into_iter().next() else {
            return;
        };
        let (Some(entry), Some(button)) = (pane.filter_entry.as_ref(), pane.filter_button.as_ref())
        else {
            return;
        };
        browser::restore_filter_controls(button, entry, filter);
        if !pane.search.replaces_listing() {
            browser::notify_filter_query(&pane.filter, &pane.filter_query, filter.query.clone());
        }
        if filter.query.trim().is_empty() {
            pane.search.show_directory_listing();
        }
    }

    /// Dismisses the filter whose field or results have focus, and returns focus to the
    /// directory cursor.
    pub fn dismiss_focused_filter(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        let Some(pane) = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane_filter_focus(pane, focused.as_ref()).is_some())
        else {
            return false;
        };
        if let Some(button) = pane.filter_button.as_ref() {
            button.set_active(false);
        }
        self.pane_navigation.borrow_mut().filter_dismissed();
        // Show the listing now rather than after the debounce, so focus lands on it.
        if pane.search.replaces_listing() {
            pane.search.flush_query();
        } else {
            browser::notify_filter_query(&pane.filter, &pane.filter_query, String::new());
        }
        if self.rename_is_active() {
            focus_pane_surface(pane);
        } else {
            self.focus_visible_pane(pane.depth);
        }
        true
    }
}
