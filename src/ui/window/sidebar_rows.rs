// SPDX-License-Identifier: MIT

//! Sidebar row buttons, their keycaps and the active-row highlight.

use gtk::prelude::*;

use crate::model::Location;

use super::{
    SidebarState, places::place_keycap, sidebar_devices::sidebar_device_shell,
    sidebar_drop_feedback,
};

impl SidebarState {
    /// Keycaps name the second key of the **g** place chord while it is armed.
    /// They stand in for the row icon so long place names are not truncated further.
    pub(super) fn add_keycap(&self, row: &gtk::Button, key: &str) -> Option<gtk::Label> {
        let content = row.child().and_downcast::<gtk::Box>()?;
        let keycap = gtk::Label::new(Some(key));
        keycap.add_css_class("sidebar-keycap");
        keycap.set_valign(gtk::Align::Center);
        content.insert_child_after(&keycap, content.first_child().as_ref());
        self.keycaps.borrow_mut().push(keycap.clone());
        Some(keycap)
    }

    pub(super) fn add_place_keycap(&self, row: &gtk::Button, step: crate::ui::keys::Command) {
        if let Some(keycap) = self.add_keycap(row, "") {
            crate::ui::shortcut_reference::follow_keys(&keycap, move |keycap| {
                keycap.set_text(&place_keycap(step));
            });
            sync_keycap(&keycap, self.keycaps_shown.get() && !self.rail.get());
        }
    }

    pub(in crate::ui) fn show_place_keycaps(&self, shown: bool) {
        self.keycaps_shown.set(shown);
        self.sync_keycaps();
    }

    pub(super) fn sync_keycaps(&self) {
        let visible = self.keycaps_shown.get() && !self.rail.get();
        for keycap in self.keycaps.borrow().iter() {
            sync_keycap(keycap, visible);
        }
    }

    pub(super) fn append_sidebar_row(&self, row: &impl IsA<gtk::Widget>) {
        if self.widget.last_child().is_some_and(|previous| {
            previous.has_css_class("sidebar-row")
                || previous.has_css_class("sidebar-device")
                || previous.has_css_class("sidebar-heading")
        }) {
            self.widget.append(&sidebar_drop_feedback::row_gap(None));
        }
        self.widget.append(row);
    }

    pub(super) fn append_place(&self, icon: &str, name: &str, location: Location) -> gtk::Button {
        let row = self.append_device_place(icon, name, location.clone(), None);
        customize_sidebar_folder_icon(&row, &location, icon);
        row
    }
}

pub(super) fn sync_keycap(keycap: &gtk::Label, visible: bool) {
    let visible = visible && !keycap.text().is_empty();
    if let Some(icon) = keycap.prev_sibling() {
        if visible && icon.is_visible() {
            // Preserve the icon's slot so neither the name nor later rows shift.
            keycap.set_size_request(icon.width(), icon.height());
        }
        icon.set_visible(!visible);
    }
    keycap.set_visible(visible);
}

/// Moves `widgets` into `container` in order, right after `anchor` or at the
/// start, without dropping keyboard focus from a row that changes parent.
pub(super) fn reparent_in_order(
    container: &gtk::Box,
    widgets: &[gtk::Widget],
    anchor: Option<&gtk::Widget>,
) {
    let mut previous = anchor.cloned();
    for widget in widgets {
        if widget.parent().as_ref() == Some(container.upcast_ref()) {
            if widget.prev_sibling() != previous {
                container.reorder_child_after(widget, previous.as_ref());
            }
        } else {
            let focused = widget
                .root()
                .and_then(|root| gtk::prelude::RootExt::focus(&root))
                .is_some_and(|focus| focus == *widget || focus.is_ancestor(widget));
            if let Some(parent) = widget.parent().and_downcast::<gtk::Box>() {
                parent.remove(widget);
            }
            container.insert_child_after(widget, previous.as_ref());
            if focused {
                widget.grab_focus();
            }
        }
        previous = Some(widget.clone());
    }
}

pub(super) fn select_sidebar_row(sidebar: &gtk::Box, selected: &gtk::Button) {
    deselect_sidebar_rows(sidebar.upcast_ref());
    set_sidebar_row_active(selected, true);
}

pub(super) fn deselect_sidebar_rows(container: &gtk::Widget) {
    let mut child = container.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(revealer) = widget.clone().downcast::<gtk::Revealer>() {
            if let Some(inner) = revealer.child() {
                deselect_sidebar_rows(&inner);
            }
        } else if let Some(row) = sidebar_row_button(&widget) {
            set_sidebar_row_active(&row, false);
        }
    }
}

pub(super) fn set_sidebar_row_active(row: &gtk::Button, active: bool) {
    if active {
        row.add_css_class("active");
    } else {
        row.remove_css_class("active");
    }
    if let Some(shell) = sidebar_device_shell(row) {
        if active {
            shell.add_css_class("active");
        } else {
            shell.remove_css_class("active");
        }
    }
}

/// Resolves the navigable row button for a sidebar child. Device rows wrap
/// their button with an eject sibling, so look one level down when the child
/// itself is a container.
pub(super) fn sidebar_row_button(widget: &gtk::Widget) -> Option<gtk::Button> {
    widget
        .clone()
        .downcast::<gtk::Button>()
        .ok()
        .filter(|button| !button.has_css_class("sidebar-heading-toggle"))
        .or_else(|| {
            widget
                .first_child()
                .and_then(|child| child.downcast::<gtk::Button>().ok())
        })
}

pub(super) fn sidebar_button(icon: &str, name: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let image = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&image);
    content.append(&label);

    let row = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Fill)
        .build();
    row.add_css_class("sidebar-row");
    row.set_cursor_from_name(Some("pointer"));
    row.set_has_frame(false);
    row
}

pub(super) fn customize_sidebar_folder_icon(row: &gtk::Button, location: &Location, icon: &str) {
    if let Some(path) = location.native_path()
        && let Some(image) = row
            .child()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Image>()
    {
        crate::ui::thumbnail::show_customized_folder_image(&image, path, icon);
    }
}
