// SPDX-License-Identifier: MIT

//! The widget tree of one file row.

use super::BoundRow;
use crate::ui::thumbnail::ThumbnailSlot;
use gtk::{glib, prelude::*};

/// A row's widgets, built on setup and found again on every bind.
pub(super) struct RowWidgets {
    pub(super) row: gtk::Box,
    pub(super) icon: ThumbnailSlot,
    pub(super) label: gtk::Label,
    pub(super) rename: gtk::Entry,
    pub(super) spacer: gtk::Box,
    pub(super) size: gtk::Label,
    pub(super) path: gtk::Label,
    pub(super) chevron: gtk::Image,
}

fn name_label() -> gtk::Label {
    gtk::Label::builder()
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build()
}

fn rename_entry() -> gtk::Entry {
    let rename = gtk::Entry::new();
    rename.add_css_class("inline-rename");
    crate::ui::accessibility::set_label(&rename, &crate::i18n::tr("Rename"));
    rename.set_hexpand(true);
    rename.set_width_chars(1);
    rename.set_visible(false);
    rename
}

fn size_label() -> gtk::Label {
    let size = gtk::Label::new(None);
    size.add_css_class("file-size");
    size.set_halign(gtk::Align::End);
    size.set_valign(gtk::Align::Fill);
    size.set_xalign(1.0);
    size.set_yalign(0.5);
    size
}

fn search_path_label() -> gtk::Label {
    let path = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .visible(false)
        .build();
    path.add_css_class("file-search-path");
    path
}

/// Sizes stay hidden while the row is renamed.
fn bind_size_visibility(size: &gtk::Label, rename: &gtk::Entry) {
    let weak_rename = rename.downgrade();
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        size,
        crate::ui::preferences::PreferenceManager::columns_show_sizes,
        move |widget, enabled| {
            let Some(label) = widget.downcast_ref::<gtk::Label>() else {
                return;
            };
            let editing = weak_rename
                .upgrade()
                .is_some_and(|entry| entry.upcast_ref::<gtk::Widget>().is_visible());
            label.set_visible(enabled && !editing && !label.label().is_empty());
        },
    );
}

impl RowWidgets {
    pub(super) fn build() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("file-row");
        let icon = ThumbnailSlot::new(17);
        icon.add_css_class("file-icon");
        icon.set_valign(gtk::Align::Center);
        let label = name_label();
        let rename = rename_entry();
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.add_css_class("file-row-spacer");
        let editor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        editor.append(&label);
        editor.append(&rename);
        editor.append(&spacer);
        let size = size_label();
        let middle = gtk::Overlay::new();
        middle.add_css_class("file-row-content");
        middle.set_hexpand(true);
        middle.set_valign(gtk::Align::Fill);
        let path = search_path_label();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.set_valign(gtk::Align::Center);
        content.append(&editor);
        content.append(&path);
        middle.set_child(Some(&content));
        middle.add_overlay(&size);
        bind_size_visibility(&size, &rename);
        let chevron = crate::assets::primary_icon(crate::assets::icons::CHEVRON_RIGHT, 15);
        chevron.add_css_class("file-chevron");
        chevron.set_valign(gtk::Align::Center);
        row.append(&icon);
        row.append(&middle);
        row.append(&chevron);
        Self {
            row,
            icon,
            label,
            rename,
            spacer,
            size,
            path,
            chevron,
        }
    }

    pub(super) fn find(item: &gtk::ListItem) -> Option<Self> {
        let row = item.child().and_downcast::<gtk::Box>()?;
        let icon = row.first_child().and_downcast::<ThumbnailSlot>()?;
        let middle = icon.next_sibling().and_downcast::<gtk::Overlay>()?;
        let content = middle.child().and_downcast::<gtk::Box>()?;
        let editor = content.first_child().and_downcast::<gtk::Box>()?;
        let path = content.last_child().and_downcast::<gtk::Label>()?;
        let label = editor.first_child().and_downcast::<gtk::Label>()?;
        let rename = label.next_sibling().and_downcast::<gtk::Entry>()?;
        let spacer = rename.next_sibling().and_downcast::<gtk::Box>()?;
        let size = middle.last_child().and_downcast::<gtk::Label>()?;
        let chevron = middle.next_sibling().and_downcast::<gtk::Image>()?;
        Some(Self {
            row,
            icon,
            label,
            rename,
            spacer,
            size,
            path,
            chevron,
        })
    }

    /// Registers the row's inline editor and returns the handles metadata fills update.
    pub(super) fn bound_row(self, item: &gtk::ListItem) -> BoundRow {
        let weak_item = glib::WeakRef::new();
        weak_item.set(Some(item));
        let weak_row = glib::WeakRef::new();
        weak_row.set(Some(&self.row));
        let weak_rename_label = glib::WeakRef::new();
        weak_rename_label.set(Some(&self.label));
        let edit = crate::ui::collection_edit::EditWidgets::new(&self.rename, &self.label);
        crate::ui::collection_edit::register_item_edit(item, &edit);
        BoundRow {
            item: weak_item,
            row: weak_row,
            rename_label: weak_rename_label,
            edit,
            spacer: self.spacer,
            size: self.size,
        }
    }

    pub(super) fn show_editing(&self, editing: bool) {
        self.rename.set_visible(editing);
        self.label.set_visible(!editing);
        self.spacer.set_visible(!editing);
    }
}
