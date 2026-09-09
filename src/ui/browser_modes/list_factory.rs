// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{
    BoundModeItem, ClickActivation, ListColumnLayout, PanePositions, PaneSection, SourceIndexMap,
    TransferHandlerSlot, assemble_list_row, entry_mode, entry_size, entry_type,
    install_list_drag_drop, install_modified_selection_click, install_preview_click,
    list_row_parts, metadata_fill_position, register_bound_mode_item, register_list_column_cell,
    set_label_if_changed, set_mode_cut_style,
};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    ui::{accessibility, browser, thumbnail},
};

pub(super) struct ListFactory {
    pub(super) browser: Weak<Browser>,
    pub(super) depth: usize,
    pub(super) positions: PanePositions,
    pub(super) selection: gtk::MultiSelection,
    pub(super) previews: Rc<Cell<bool>>,
    pub(super) activation: Rc<Cell<ClickActivation>>,
    pub(super) transfers: TransferHandlerSlot,
    pub(super) cuts: Rc<RefCell<HashSet<Location>>>,
    pub(super) columns: ListColumnLayout,
    pub(super) scrolling: Rc<Cell<bool>>,
    pub(super) bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    pub(super) state: Option<Weak<crate::ui::browser::ViewState>>,
    pub(super) filter_query: Rc<RefCell<String>>,
}

impl ListFactory {
    pub(super) fn build(self) -> gtk::SignalListItemFactory {
        let context = Rc::new(self);
        let factory = gtk::SignalListItemFactory::new();
        let setup = context.clone();
        factory.connect_setup(move |_, item| setup.setup(item));
        factory.connect_bind(move |_, item| context.bind(item));
        factory.connect_unbind(|_, item| thumbnail::cancel_list_item_thumbnails(item));
        factory
    }

    fn setup(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let widget = assemble_list_row();
        if !self.scrolling.get() {
            animate_appearance(&widget);
        }
        let Some(row) = ListRow::from_widget(widget) else {
            return;
        };
        row.register_columns(&self.columns);
        self.install_interactions(item, &row);
        item.set_child(Some(&row.widget));
        register_bound_mode_item(&self.bound_items, item, &row.widget, &row.name);
    }

    fn install_interactions(&self, item: &gtk::ListItem, row: &ListRow) {
        install_preview_click(
            &row.widget,
            item,
            self.browser.clone(),
            self.previews.clone(),
            self.activation.clone(),
            self.depth,
            Some((self.positions.index.clone(), self.positions.view.clone())),
            self.filter_query.clone(),
        );
        let content_click = install_modified_selection_click(
            &row.widget,
            item,
            self.selection.clone(),
            self.browser.clone(),
            self.depth,
            self.positions.clone(),
        );
        install_list_drag_drop(
            &row.widget,
            item,
            self.browser.clone(),
            self.transfers.clone(),
            self.depth,
            Some((self.positions.index.clone(), self.positions.view.clone())),
            (Some(row.name.upcast_ref()), &content_click),
        );
    }

    fn binding(&self, item: &gtk::ListItem) -> Option<ListBinding> {
        let position = item
            .item()
            .and_then(|value| self.positions.index.of_item(&value))?;
        let browser = self.browser.upgrade()?;
        let entry = browser.entry_at(self.depth, position)?;
        Some(ListBinding {
            browser,
            depth: self.depth,
            position,
            entry,
        })
    }

    fn bind(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item
            .child()
            .and_downcast::<gtk::Box>()
            .and_then(ListRow::from_widget)
        else {
            return;
        };
        let Some(binding) = self.binding(item) else {
            row.clear();
            return;
        };
        let pending_name = self
            .state
            .as_ref()
            .and_then(Weak::upgrade)
            .and_then(|state| state.pending_rename_name(&binding.entry));
        row.bind_labels(item, &binding.entry, pending_name.as_deref());
        if self.scrolling.get() {
            set_label_if_changed(&row.modified, &crate::util::modified_date(&binding.entry));
        } else {
            set_mode_cut_style(
                &row.widget,
                self.cuts.borrow().contains(&binding.entry.location),
            );
            binding.refresh_details(&row);
        }
    }
}

fn animate_appearance(row: &gtk::Box) {
    row.add_css_class("file-appear");
    let weak = row.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(row) = weak.upgrade() {
            row.remove_css_class("file-appear");
        }
    });
}

struct ListRow {
    widget: gtk::Box,
    name_cell: gtk::Widget,
    icon: thumbnail::ThumbnailSlot,
    name: gtk::Label,
    field: gtk::Entry,
    mode: gtk::Label,
    size: gtk::Label,
    kind: gtk::Label,
    modified: gtk::Label,
}

impl ListRow {
    fn from_widget(widget: gtk::Box) -> Option<Self> {
        let (icon, name, field, mode, size, kind, modified) = list_row_parts(&widget)?;
        let name_cell = widget.first_child()?;
        Some(Self {
            widget,
            name_cell,
            icon,
            name,
            field,
            mode,
            size,
            kind,
            modified,
        })
    }

    fn register_columns(&self, columns: &ListColumnLayout) {
        for (index, widget) in [
            self.name_cell.clone(),
            self.mode.clone().upcast(),
            self.size.clone().upcast(),
            self.kind.clone().upcast(),
            self.modified.clone().upcast(),
        ]
        .into_iter()
        .enumerate()
        {
            register_list_column_cell(columns, index, &widget);
        }
    }

    fn bind_labels(&self, item: &gtk::ListItem, entry: &FileEntry, pending_name: Option<&str>) {
        self.name.set_visible(true);
        self.field.set_visible(false);
        set_label_if_changed(&self.name, pending_name.unwrap_or(&entry.display_name));
        set_label_if_changed(&self.mode, &entry_mode(entry));
        set_label_if_changed(&self.size, &entry_size(entry));
        set_label_if_changed(&self.kind, entry_type(entry));
        accessibility::describe_entry(
            item,
            pending_name.unwrap_or(&entry.display_name),
            Some(entry),
        );
    }

    fn clear(&self) {
        self.widget.remove_css_class("cut-item");
        thumbnail::show_fallback_icon(&self.icon, crate::assets::icons::DOCUMENTS, 18);
        self.name.set_label("");
        self.name.set_visible(true);
        self.field.set_visible(false);
        self.mode.set_label("");
        self.size.set_label("");
        self.kind.set_label("");
        crate::util::set_modified_date(&self.modified, None, "");
    }
}

/// An owned source snapshot; GTK updates and metadata requests run after lookup borrows end.
struct ListBinding {
    browser: Rc<Browser>,
    depth: usize,
    position: usize,
    entry: FileEntry,
}

impl ListBinding {
    /// Settling must not reset labels or an active rename editor.
    fn refresh_details(&self, row: &ListRow) {
        thumbnail::set_thumbnail_or_icon(
            &row.icon,
            &self.entry,
            browser::entry_icon(&self.entry),
            18,
            18,
        );
        if let Some(position) = metadata_fill_position(Some(self.position), &self.entry, true) {
            self.browser
                .request_metadata_fill(self.depth, position, self.entry.location.clone());
        }
        crate::util::set_modified_date(&row.modified, Some(&self.entry), "—");
    }
}

pub(super) fn refresh_list_section(
    browser: &Rc<Browser>,
    depth: usize,
    source_index: &SourceIndexMap,
    section: &PaneSection,
    cuts: &HashSet<Location>,
) {
    section.bound_items.borrow().iter().for_each(|bound| {
        let Some(row) = bound
            .widget
            .upgrade()
            .and_downcast::<gtk::Box>()
            .and_then(ListRow::from_widget)
        else {
            return;
        };
        let Some(item) = bound.item.upgrade() else {
            return;
        };
        let Some(position) = item.item().and_then(|value| source_index.of_item(&value)) else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        set_mode_cut_style(&row.widget, cuts.contains(&entry.location));
        ListBinding {
            browser: browser.clone(),
            depth,
            position,
            entry,
        }
        .refresh_details(&row);
    });
}

#[cfg(test)]
mod tests;
