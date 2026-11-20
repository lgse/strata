// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{
    BoundModeItem, ClickActivation, ListColumnLayout, PanePositions, PaneSection, SourceIndexMap,
    TransferHandlerSlot, assemble_list_row, install_list_drag_drop,
    install_modified_selection_click, install_preview_click, metadata_fill_position,
    register_bound_mode_item, register_list_column_cell, set_label_if_changed, set_mode_mark_style,
};
use crate::{
    app::Browser,
    model::FileEntry,
    ui::{
        accessibility,
        browser::{self, ClipboardMark, ClipboardMarks, mark_in},
        preferences::ListColumnId,
        thumbnail,
    },
};

pub(super) struct ListFactory {
    pub(super) browser: Weak<Browser>,
    pub(super) depth: usize,
    pub(super) positions: PanePositions,
    pub(super) selection: gtk::MultiSelection,
    pub(super) syncing: Rc<Cell<bool>>,
    pub(super) previews: Rc<Cell<bool>>,
    pub(super) activation: Rc<Cell<ClickActivation>>,
    pub(super) transfers: TransferHandlerSlot,
    pub(super) marks: Rc<RefCell<ClipboardMarks>>,
    pub(super) columns: ListColumnLayout,
    pub(super) scrolling: Rc<Cell<bool>>,
    pub(super) bound_items: Rc<RefCell<Vec<BoundModeItem>>>,
    pub(super) state: Option<Weak<crate::ui::browser::ViewState>>,
    pub(super) filter_query: Rc<RefCell<String>>,
    pub(super) enter_target: Option<crate::ui::enter_target::EnterTarget>,
}

impl ListFactory {
    pub(super) fn build(self) -> gtk::SignalListItemFactory {
        let setup_items = self.bound_items.clone();
        let context = Rc::new(self);
        let factory = gtk::SignalListItemFactory::new();
        let setup = context.clone();
        factory.connect_setup(move |_, item| setup.setup(item));
        factory.connect_bind(move |_, item| context.bind(item));
        super::install_edit_unbind(&factory, &setup_items);
        factory
    }

    fn setup(&self, object: &glib::Object) {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let widget = assemble_list_row();
        let Some(row) = ListRow::from_widget(widget) else {
            return;
        };
        row.register_columns(&self.columns);
        self.install_interactions(item, &row);
        item.set_child(Some(&row.widget));
        let edit = crate::ui::collection_edit::EditWidgets::new(&row.field, &row.name);
        register_bound_mode_item(&self.bound_items, item, &row.widget, &row.name, edit);
    }

    fn install_interactions(&self, item: &gtk::ListItem, row: &ListRow) {
        super::install_folder_peek(
            &row.widget,
            item,
            self.state.clone(),
            self.browser.clone(),
            self.positions.index.clone(),
            self.positions.view.clone(),
            self.depth,
        );
        let slow_click = Rc::new(super::SlowClickRename::default());
        let activation_click = install_preview_click(
            &row.widget,
            item,
            &row.name,
            self.browser.clone(),
            self.state.clone().unwrap_or_default(),
            self.previews.clone(),
            self.activation.clone(),
            self.depth,
            Some((self.positions.index.clone(), self.positions.view.clone())),
            self.filter_query.clone(),
            slow_click.clone(),
        );
        let content_click = install_modified_selection_click(
            &row.widget,
            item,
            self.selection.clone(),
            self.syncing.clone(),
            self.browser.clone(),
            self.state.clone().unwrap_or_default(),
            self.depth,
            self.positions.clone(),
            slow_click.clone(),
        );
        activation_click.group_with(&content_click);
        install_list_drag_drop(
            &row.widget,
            item,
            self.browser.clone(),
            self.transfers.clone(),
            self.depth,
            Some((self.positions.index.clone(), self.positions.view.clone())),
            self.state.clone(),
            (&content_click, true, slow_click),
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
        let Some(widget) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        if let Some(enter_target) = &self.enter_target {
            enter_target.decorate(&widget, item);
        }
        let Some(row) = ListRow::from_widget(widget) else {
            return;
        };
        let Some(binding) = self.binding(item) else {
            row.clear();
            return;
        };
        let state = self.state.as_ref().and_then(Weak::upgrade);
        let pending_name = state
            .as_ref()
            .and_then(|state| state.pending_rename_name(&binding.entry));
        let edit = super::bound_edit(item);
        if let Some(edit) = &edit {
            edit.bind(&binding.entry.location);
        }
        row.bind_labels(item, &binding.entry, pending_name.as_deref());
        crate::ui::browser::find::highlight_listing_name(
            row.name.upcast_ref(),
            state
                .as_ref()
                .and_then(|state| state.find_highlight())
                .as_deref(),
            &self.filter_query.borrow(),
        );
        if let Some(edit) = &edit {
            edit.display.set_visible(!edit.is_editing());
            edit.field.set_visible(edit.is_editing());
        }
        if self.scrolling.get() {
            binding.request_thumbnail_and_metadata(&row);
            row.update_dates(Some(&binding.entry), false);
        } else {
            let mark = mark_in(&self.marks.borrow(), &binding.entry.location);
            set_mode_mark_style(&row.widget, mark);
            binding.refresh_details(&row);
        }
        row.icon.set_hidden(binding.entry.is_hidden);
        row.icon.set_base_opacity(1.0);
    }
}

pub(super) struct ListRow {
    pub(super) widget: gtk::Box,
    pub(super) name_cell: gtk::Widget,
    pub(super) icon: thumbnail::ThumbnailSlot,
    pub(super) name: gtk::Label,
    pub(super) field: gtk::Entry,
    labels: [Option<gtk::Label>; ListColumnId::ALL.len()],
}

impl ListRow {
    pub(super) fn from_widget(widget: gtk::Box) -> Option<Self> {
        let mut name_cell = None;
        let mut labels = std::array::from_fn(|_| None);
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if current.has_css_class(ListColumnId::Name.css_class()) {
                name_cell = Some(current);
            } else if let Some(column) = ListColumnId::ALL
                .into_iter()
                .find(|column| current.has_css_class(column.css_class()))
            {
                labels[column.index()] = current.downcast().ok();
            }
        }
        let name_cell = name_cell?;
        let icon = name_cell
            .first_child()?
            .downcast::<thumbnail::ThumbnailSlot>()
            .ok()?;
        let name = icon.next_sibling()?.downcast::<gtk::Label>().ok()?;
        let field = name.next_sibling()?.downcast::<gtk::Entry>().ok()?;
        Some(Self {
            widget,
            name_cell,
            icon,
            name,
            field,
            labels,
        })
    }

    pub(super) fn cell(&self, column: ListColumnId) -> Option<gtk::Widget> {
        if column == ListColumnId::Name {
            Some(self.name_cell.clone())
        } else {
            self.labels[column.index()].clone().map(gtk::Label::upcast)
        }
    }

    fn register_columns(&self, columns: &ListColumnLayout) {
        for column in ListColumnId::ALL {
            if let Some(cell) = self.cell(column) {
                register_list_column_cell(columns, column.index(), &cell);
            }
        }
    }

    fn has_media_column(&self) -> bool {
        ListColumnId::ALL.into_iter().any(|column| {
            column
                .definition()
                .sort_key
                .is_some_and(|key| key.uses_media_metadata())
                && self.labels[column.index()]
                    .as_ref()
                    .is_some_and(gtk::Label::is_visible)
        })
    }

    pub(super) fn update_metadata(&self, entry: &FileEntry) {
        self.update_text_columns(Some(entry));
        self.update_dates(Some(entry), true);
    }

    fn update_text_columns(&self, entry: Option<&FileEntry>) {
        for (column, label) in ListColumnId::ALL.into_iter().zip(&self.labels) {
            if let Some(label) = label {
                let text = entry.and_then(|entry| super::metadata::list_cell_text(column, entry));
                if let Some(text) = text.or_else(|| entry.is_none().then(String::new)) {
                    set_label_if_changed(label, &text);
                }
            }
        }
    }

    fn update_dates(&self, entry: Option<&FileEntry>, live: bool) {
        for (column, label) in ListColumnId::ALL.into_iter().zip(&self.labels) {
            if let Some(label) = label {
                super::metadata::update_list_date(column, label, entry, live);
            }
        }
    }

    fn bind_labels(&self, item: &gtk::ListItem, entry: &FileEntry, pending_name: Option<&str>) {
        set_label_if_changed(&self.name, pending_name.unwrap_or(&entry.display_name));
        self.name
            .set_opacity(if entry.is_hidden { 0.65 } else { 1.0 });
        self.update_text_columns(Some(entry));
        accessibility::describe_entry(
            item,
            pending_name.unwrap_or(&entry.display_name),
            Some(entry),
        );
    }

    fn clear(&self) {
        crate::ui::file_providers::bind(&self.icon, None);
        set_mode_mark_style(&self.widget, ClipboardMark::None);
        thumbnail::show_fallback_icon(&self.icon, crate::assets::icons::DOCUMENTS, 18);
        self.icon.set_hidden(false);
        self.icon.set_base_opacity(1.0);
        self.name.set_label("");
        self.name.set_opacity(1.0);
        self.name.set_visible(true);
        self.field.set_visible(false);
        self.update_text_columns(None);
        self.update_dates(None, true);
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
    /// Detail updates must not reset labels or an active rename editor.
    fn refresh_details(&self, row: &ListRow) {
        self.request_thumbnail_and_metadata(row);
        row.update_dates(Some(&self.entry), true);
    }

    fn request_thumbnail_and_metadata(&self, row: &ListRow) {
        thumbnail::set_thumbnail_or_icon(
            &row.icon,
            &self.entry,
            browser::entry_icon(&self.entry),
            18,
            18,
        );
        self.request_metadata(row);
    }

    fn request_metadata(&self, row: &ListRow) {
        let details = row.has_media_column();
        if let Some(position) =
            metadata_fill_position(Some(self.position), &self.entry, true, details)
        {
            thumbnail::request_metadata(
                &row.icon,
                &row.widget,
                &self.browser,
                self.depth,
                position,
                self.entry.location.clone(),
                details || self.entry.is_directory(),
            );
        }
    }
}

pub(super) fn refresh_list_section(
    browser: &Rc<Browser>,
    depth: usize,
    source_index: &SourceIndexMap,
    section: &PaneSection,
    marks: &ClipboardMarks,
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
        if !thumbnail::near_viewport(&row.widget) {
            return;
        }
        let Some(item) = bound.item.upgrade() else {
            return;
        };
        let Some(position) = item.item().and_then(|value| source_index.of_item(&value)) else {
            return;
        };
        let Some(entry) = browser.entry_at(depth, position) else {
            return;
        };
        let is_hidden = entry.is_hidden;
        set_mode_mark_style(&row.widget, mark_in(marks, &entry.location));
        row.name.set_opacity(if is_hidden { 0.65 } else { 1.0 });
        row.update_dates(Some(&entry), true);
        row.icon.set_hidden(is_hidden);
        row.icon.set_base_opacity(1.0);
    });
}

pub(super) fn request_visible_list_details(browser: &Rc<Browser>, pane: &super::pane::Pane) {
    for section in pane.item_sections() {
        for bound in section.bound_items.borrow().iter() {
            let Some(row) = bound
                .widget
                .upgrade()
                .and_downcast::<gtk::Box>()
                .and_then(ListRow::from_widget)
            else {
                continue;
            };
            let Some(position) = bound
                .item
                .upgrade()
                .and_then(|item| item.item())
                .and_then(|item| pane.source_index.of_item(&item))
            else {
                continue;
            };
            if let Some(entry) = browser.entry_at(pane.depth, position) {
                ListBinding {
                    browser: browser.clone(),
                    depth: pane.depth,
                    position,
                    entry,
                }
                .request_metadata(&row);
            }
        }
    }
}
