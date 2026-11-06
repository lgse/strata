// SPDX-License-Identifier: MIT

//! Binding a file row to the entry or search hit at its position.

use super::super::{ColumnHits, column_size_text, set_active_path_style, set_mark_path_style};
use super::{RowContext, widgets::RowWidgets};
use crate::model::FileEntry;
use crate::ui::browser::{
    ViewState,
    clipboard::{ClipboardMark, clipboard_mark},
    entry::{entry_icon, metadata_needs_fill, model_display_name},
};
use crate::ui::browser_modes::BrowserMode;
use crate::ui::enter_target::EnterTarget;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// What the column's bind handler reads beyond the rows' own context.
pub(super) struct BindContext {
    pub(super) rows: Rc<RowContext>,
    pub(super) hits: Rc<RefCell<ColumnHits>>,
    pub(super) enter_target: EnterTarget,
    pub(super) scrolling: Rc<Cell<bool>>,
}

fn set_css_class(widget: &impl IsA<gtk::Widget>, class: &str, enabled: bool) {
    if enabled {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// One bind of a row to its current position.
struct RowBinding<'a> {
    context: &'a BindContext,
    item: &'a gtk::ListItem,
    widgets: RowWidgets,
    state: Option<Rc<ViewState>>,
    entry: Option<FileEntry>,
    searching: bool,
    source_position: Option<usize>,
}

impl BindContext {
    pub(super) fn bind(&self, item: &gtk::ListItem) {
        let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let Some(widgets) = RowWidgets::find(item) else {
            return;
        };
        let rows = &self.rows;
        let state = rows.state.upgrade();
        widgets.row.remove_css_class("keyboard-cursor");
        if state
            .as_ref()
            .is_some_and(|state| self.is_keyboard_cursor(state, item))
        {
            widgets.row.add_css_class("keyboard-cursor");
        }
        widgets.label.set_label(model_display_name(&value.string()));
        let searching = rows.search_active.get();
        set_css_class(&widgets.row, "filter-result", searching);
        self.enter_target.decorate(&widgets.row, item);
        let source_position = (!searching)
            .then(|| rows.map.source_position(item.position()))
            .flatten();
        let entry = if searching {
            rows.search_entry(item.position())
        } else {
            source_position.and_then(|position| {
                state
                    .as_ref()
                    .and_then(|state| state.browser.entry_at(rows.depth, position))
            })
        };
        RowBinding {
            context: self,
            item,
            widgets,
            state,
            entry,
            searching,
            source_position,
        }
        .apply();
    }

    fn is_keyboard_cursor(&self, state: &ViewState, item: &gtk::ListItem) -> bool {
        let depth = self.rows.depth;
        state.focused_column_depth() == Some(depth)
            && state
                .browser
                .focused_item()
                .is_some_and(|(focused_depth, position, _)| {
                    focused_depth == depth
                        && self.rows.map.view_position(position) == Some(item.position())
                })
    }
}

impl RowBinding<'_> {
    fn depth(&self) -> usize {
        self.context.rows.depth
    }

    fn apply(self) {
        let editing = self.bind_edit();
        self.widgets.show_editing(editing);
        self.show_name();
        self.highlight_name();
        self.show_origin();
        self.style_paths();
        crate::ui::file_providers::bind(
            &self.widgets.icon,
            self.entry.as_ref().and_then(|e| e.location.native_path()),
        );
        let show_sizes = crate::ui::preferences::PreferenceManager::shared().columns_show_sizes();
        if let Some(entry) = self.entry.as_ref() {
            self.show_entry_icon(entry, show_sizes);
        } else {
            self.show_placeholder_icon();
        }
        let size = &self.widgets.size;
        let size_text = column_size_text(self.entry.as_ref());
        size.set_label(&size_text);
        size.set_visible(show_sizes && !editing && !size_text.is_empty());
        crate::ui::accessibility::describe_entry(
            self.item,
            &self.widgets.label.label(),
            self.entry.as_ref(),
        );
    }

    /// Binds the row's inline editor and reports whether it is renaming.
    fn bind_edit(&self) -> bool {
        let edit = crate::ui::collection_edit::item_edit(self.item);
        if let Some(edit) = &edit {
            if let Some(entry) = &self.entry {
                edit.bind(&entry.location);
            } else {
                edit.unbind();
            }
        }
        edit.as_ref().is_some_and(|edit| edit.is_editing())
    }

    fn show_name(&self) {
        let label = &self.widgets.label;
        let Some(entry) = self.entry.as_ref() else {
            label.set_opacity(1.0);
            return;
        };
        let pending_name = self
            .state
            .as_ref()
            .and_then(|state| state.pending_rename_name(entry));
        label.set_label(pending_name.as_deref().unwrap_or(&entry.display_name));
        label.set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
    }

    fn highlight_name(&self) {
        let label = self.widgets.label.upcast_ref();
        let find = self.state.as_ref().and_then(|state| state.find_highlight());
        if !self.searching {
            crate::ui::browser::find::highlight_listing_name(
                label,
                find.as_deref(),
                &self.context.rows.map.query(),
            );
            return;
        }
        let hits = self.context.hits.borrow();
        let hit = self
            .context
            .rows
            .search_results
            .borrow()
            .get(self.item.position() as usize)
            .and_then(|hit| hits.ranges.get(&hit.path).cloned());
        crate::ui::browser::find::highlight_hit_name(label, find.as_deref(), hit.as_deref());
    }

    /// A search hit shows the folder it was found in.
    fn show_origin(&self) {
        let row = &self.widgets.row;
        let path = &self.widgets.path;
        let origin = self
            .entry
            .as_ref()
            .filter(|_| self.searching)
            .map(|entry| entry.location.display_path());
        path.set_label(origin.as_deref().unwrap_or_default());
        path.set_visible(origin.is_some() && self.context.hits.borrow().recursive);
        row.set_widget_name(origin.as_deref().unwrap_or("file-row"));
        crate::ui::accessibility::set_description(row, origin.as_deref());
    }

    fn style_paths(&self) {
        let depth = self.depth();
        let browser = self.state.as_ref().map(|state| &state.browser);
        let active = self.entry.as_ref().is_some_and(|entry| {
            browser
                .as_ref()
                .is_some_and(|browser| browser.is_open_child(depth, &entry.location))
        });
        let immediate =
            browser.as_ref().and_then(|browser| browser.active_depth()) == Some(depth + 1);
        set_active_path_style(&self.widgets.row, active, immediate);
        set_mark_path_style(
            &self.widgets.row,
            self.entry
                .as_ref()
                .map_or(ClipboardMark::None, |entry| clipboard_mark(&entry.location)),
        );
    }

    fn mode_active(&self) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.mode_views.borrow().mode() == BrowserMode::Columns)
    }

    fn show_entry_icon(&self, entry: &FileEntry, show_sizes: bool) {
        let icon = &self.widgets.icon;
        let mode_active = self.mode_active();
        let search_file = self.searching && mode_active && !entry.is_directory();
        if search_file {
            // Search results have no directory metadata-fill producer.
            crate::ui::thumbnail::set_thumbnail_or_icon_for_path(
                icon,
                entry.local_thumbnail_path().expect("local search result"),
                entry_icon(entry),
                17,
                17,
            );
        } else if entry.is_directory() || mode_active {
            crate::ui::thumbnail::set_thumbnail_or_icon(icon, entry, entry_icon(entry), 17, 17);
        } else {
            crate::ui::thumbnail::show_fallback_icon(icon, entry_icon(entry), 17);
        }
        icon.set_hidden(entry.is_hidden);
        icon.set_base_opacity(if entry.is_directory() { 1.0 } else { 0.72 });
        self.widgets.chevron.set_visible(entry.is_directory());
        if mode_active {
            self.request_metadata(entry, show_sizes);
        }
    }

    fn request_metadata(&self, entry: &FileEntry, show_sizes: bool) {
        let folder_size = show_sizes && entry.is_directory();
        let wants_fill =
            !self.context.scrolling.get() && (metadata_needs_fill(entry) || folder_size);
        if wants_fill
            && let Some(state) = self.state.as_ref()
            && let Some(position) = self.source_position
        {
            crate::ui::thumbnail::request_metadata(
                &self.widgets.icon,
                &self.widgets.row,
                &state.browser,
                self.depth(),
                position,
                entry.location.clone(),
                folder_size,
            );
        }
    }

    fn show_placeholder_icon(&self) {
        let icon = &self.widgets.icon;
        crate::ui::thumbnail::show_fallback_icon(icon, crate::assets::icons::DOCUMENTS, 17);
        icon.set_hidden(false);
        icon.set_mark(ClipboardMark::None);
        icon.set_base_opacity(0.72);
        self.widgets.chevron.set_visible(false);
    }
}
