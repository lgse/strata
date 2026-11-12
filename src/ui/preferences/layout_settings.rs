// SPDX-License-Identifier: MIT

//! Column views and list columns: widths, order, visibility, and sorting.

use super::*;

impl PreferenceManager {
    pub fn compact_full_names(&self) -> bool {
        self.preferences.borrow().compact_full_names
    }

    pub fn set_compact_full_names(&self, enabled: bool) {
        self.preferences.borrow_mut().compact_full_names = enabled;
        self.save_preferences();
    }

    pub fn columns_show_sizes(&self) -> bool {
        self.preferences.borrow().columns_show_sizes
    }

    pub fn set_columns_show_sizes(&self, enabled: bool) {
        self.preferences.borrow_mut().columns_show_sizes = enabled;
        self.save_preferences();
    }

    pub fn columns_mirror_selection(&self) -> bool {
        self.preferences.borrow().columns_mirror_selection
    }

    pub fn set_columns_mirror_selection(&self, enabled: bool) {
        self.preferences.borrow_mut().columns_mirror_selection = enabled;
        self.save_preferences();
    }

    pub fn chooser_column_width(&self) -> Option<i32> {
        self.preferences.borrow().chooser_column_width
    }

    pub fn set_chooser_column_width(&self, width: Option<i32>) {
        self.preferences.borrow_mut().chooser_column_width = width.filter(|width| *width > 0);
        self.save_preferences();
    }

    pub fn browser_column_width(&self) -> Option<i32> {
        self.preferences.borrow().browser_column_width
    }

    pub fn set_browser_column_width(&self, width: Option<i32>) {
        self.preferences.borrow_mut().browser_column_width = width.filter(|width| *width > 0);
        self.save_preferences();
    }

    pub fn browser_list_columns(&self) -> Option<ListColumns> {
        self.preferences.borrow().browser_list_columns
    }

    pub fn set_browser_list_columns(&self, columns: Option<ListColumns>) {
        self.preferences.borrow_mut().browser_list_columns = columns.filter(ListColumns::is_valid);
        self.save_preferences();
    }

    pub fn chooser_list_columns(&self) -> Option<ListColumns> {
        self.preferences.borrow().chooser_list_columns
    }

    pub fn set_chooser_list_columns(&self, columns: Option<ListColumns>) {
        self.preferences.borrow_mut().chooser_list_columns = columns.filter(ListColumns::is_valid);
        self.save_preferences();
    }

    pub fn list_column_order(&self) -> Vec<ListColumnId> {
        sanitize_list_column_order(&self.preferences.borrow().list_column_order)
    }

    pub fn set_list_column_order(&self, order: Vec<ListColumnId>) {
        self.preferences.borrow_mut().list_column_order = sanitize_list_column_order(&order);
        self.save_preferences();
    }

    pub fn list_hidden_columns(&self) -> Vec<ListColumnId> {
        sanitize_list_hidden_columns(&self.preferences.borrow().list_hidden_columns)
    }

    pub fn set_list_hidden_columns(&self, hidden: Vec<ListColumnId>) {
        self.preferences.borrow_mut().list_hidden_columns = sanitize_list_hidden_columns(&hidden);
        self.save_preferences();
    }

    pub fn is_list_column_visible(&self, column: ListColumnId) -> bool {
        if column == ListColumnId::Name {
            return true;
        }
        !self.list_hidden_columns().contains(&column)
    }

    pub fn set_list_column_visible(&self, column: ListColumnId, visible: bool) {
        if column == ListColumnId::Name {
            return;
        }
        let mut hidden = self.list_hidden_columns();
        if visible {
            hidden.retain(|col| *col != column);
        } else if !hidden.contains(&column) {
            hidden.push(column);
        }
        self.set_list_hidden_columns(hidden);
    }

    pub fn move_list_column(&self, column: ListColumnId, target: ListColumnId, after: bool) {
        let mut order = self.list_column_order();
        let Some(from) = order.iter().position(|item| *item == column) else {
            return;
        };
        let Some(target) = order.iter().position(|item| *item == target) else {
            return;
        };
        let mut to = target + usize::from(after);
        if from < to {
            to -= 1;
        }
        if from != to {
            let column = order.remove(from);
            order.insert(to, column);
            self.set_list_column_order(order);
        }
    }

    pub fn reset_list_columns(&self) {
        {
            let mut prefs = self.preferences.borrow_mut();
            prefs.list_column_order = default_list_column_order();
            prefs.list_hidden_columns.clear();
        }
        self.save_preferences();
    }

    pub fn list_visible_columns(&self) -> Vec<ListColumnId> {
        let hidden = self.list_hidden_columns();
        self.list_column_order()
            .into_iter()
            .filter(|col| *col == ListColumnId::Name || !hidden.contains(col))
            .collect()
    }

    pub fn sort_preferences(&self) -> ViewPreferences {
        sort_preferences(&self.preferences.borrow())
    }

    /// Unlike `set_default_sort`, this keeps folder sorts that equal the new
    /// default: with per-folder sorts off, they are kept but ignored.
    pub fn set_sort_preferences(&self, preferences: ViewPreferences) {
        if preferences.sort_key == SortKey::Recency {
            return;
        }
        let mut stored = self.preferences.borrow_mut();
        stored.show_hidden = preferences.show_hidden;
        stored.folders_first = preferences.folders_first;
        store_sort(&mut stored, preferences.sort_options());
        drop(stored);
        self.save_preferences();
    }
}
