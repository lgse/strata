// SPDX-License-Identifier: GPL-3.0-or-later

//! Accessible semantics for browser widgets whose nested content has no useful derived name.

use gtk::prelude::*;

use crate::model::{EntryKind, FileEntry};

use super::browser_modes::BrowserMode;

pub(super) fn set_label(widget: &impl IsA<gtk::Accessible>, label: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(label)]);
}

/// The name belongs on the list item rather than on the row content: the item
/// is the widget carrying the `list item` / `table cell` accessible role and
/// the selected and focused states.
pub(super) fn describe_entry(item: &gtk::ListItem, display_name: &str, entry: Option<&FileEntry>) {
    item.set_accessible_label(display_name);
    item.set_accessible_description(entry.map_or("Entry", entry_kind_name));
}

/// A plain `GtkBox` has the `generic` accessible role, and ARIA forbids naming
/// those, so GTK silently drops any label set on one.
pub(super) fn pane_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Group)
        .build()
}

pub(super) fn dialog_box(title: &str) -> gtk::Box {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Dialog)
        .build();
    content.update_property(&[gtk::accessible::Property::Label(title)]);
    content.update_state(&[gtk::accessible::State::Hidden(false)]);
    content
}

/// The pane, not the entry list, carries this: an empty directory replaces its
/// list with a placeholder, and the pane has to stay identifiable either way.
pub(super) fn describe_pane(pane: &impl IsA<gtk::Accessible>, directory: &str, mode: BrowserMode) {
    pane.update_property(&[
        gtk::accessible::Property::Label(directory),
        gtk::accessible::Property::Description(view_name(mode)),
    ]);
}

pub(super) const ENTRY_CONTAINER_DESCRIPTION: &str = "Files";

pub(super) fn describe_entry_container(container: &impl IsA<gtk::Accessible>, directory: &str) {
    container.update_property(&[
        gtk::accessible::Property::Label(directory),
        gtk::accessible::Property::Description(ENTRY_CONTAINER_DESCRIPTION),
    ]);
}

pub(super) fn menu_item_button() -> gtk::Button {
    gtk::Button::builder()
        .accessible_role(gtk::AccessibleRole::MenuItem)
        .has_frame(false)
        .build()
}

pub(super) fn menu_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Menu)
        .build()
}

/// Names a menu item after its action and announces its accelerator
/// separately, so the shortcut text does not become part of the name.
pub(super) fn describe_menu_item(
    button: &impl IsA<gtk::Accessible>,
    label: &str,
    accelerator: &str,
) {
    button.update_property(&[
        gtk::accessible::Property::Label(label),
        gtk::accessible::Property::Description(accelerator),
    ]);
}

pub(super) fn view_name(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => "Columns view",
        BrowserMode::Icons => "Icons view",
        BrowserMode::List => "List view",
    }
}

fn entry_kind_name(entry: &FileEntry) -> &'static str {
    match entry.kind {
        EntryKind::Directory => "Folder",
        EntryKind::DirectorySymbolicLink => "Folder link",
        EntryKind::File => "File",
        EntryKind::FileSymbolicLink => "File link",
        EntryKind::SymbolicLink => "Broken link",
        EntryKind::Other => "Other",
    }
}

#[cfg(test)]
mod tests;
