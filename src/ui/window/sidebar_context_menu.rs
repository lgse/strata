// SPDX-License-Identifier: MIT

//! The context menu attached to sidebar places.

use std::rc::Rc;

use gtk::prelude::*;

use crate::model::Location;

use super::{SidebarState, sidebar};

impl SidebarState {
    pub(super) fn attach_place_context_menu(
        self: &Rc<Self>,
        row: &gtk::Button,
        location: Location,
        on_unpin: impl Fn(&Rc<Self>) + 'static,
    ) {
        self.attach_place_context_menu_with(row, location, on_unpin, None);
    }

    /// Like [`Self::attach_place_context_menu`], with `extra` above the standard items.
    pub(super) fn attach_place_context_menu_with(
        self: &Rc<Self>,
        row: &gtk::Button,
        location: Location,
        on_unpin: impl Fn(&Rc<Self>) + 'static,
        extra: Option<PlaceMenuExtra>,
    ) {
        let menu = crate::ui::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let extra_item = extra.map(|extra| {
            let button = sidebar_context_option(extra.icon, extra.label, false);
            menu.append(&button);
            (button, extra.action)
        });
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&unpin);
        let customize = location.native_path().map(|path| {
            let button = sidebar_context_option(crate::assets::icons::PALETTE, "Customize…", false);
            menu.append(&button);
            (button, path.to_path_buf())
        });
        menu.append(&properties);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(row);
        sidebar::append_open_in_tab(&menu, &popover, row, &self.view, &location);

        if let Some((button, action)) = extra_item {
            let popover = popover.downgrade();
            let weak_state = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                if let Some(state) = weak_state.upgrade() {
                    action(&state);
                }
            });
        }

        if let Some((customize, path)) = customize {
            let weak_popover = popover.downgrade();
            let weak_row = row.downgrade();
            customize.connect_clicked(move |_| {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                if let Some(row) = weak_row.upgrade() {
                    crate::ui::browser::show_customize_modal(
                        &row,
                        path.clone(),
                        true,
                        crate::assets::icons::FOLDER,
                    );
                }
            });
        }

        let weak_state = Rc::downgrade(self);
        let unpin_popover = popover.downgrade();
        unpin.connect_clicked(move |_| {
            if let Some(popover) = unpin_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                on_unpin(&state);
            }
        });
        let properties_view = self.view.downgrade();
        let properties_location = location;
        let properties_popover = popover.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            if let Some(view) = properties_view.upgrade() {
                view.show_location_properties(&properties_location);
            }
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        });
        row.add_controller(context);
    }
}

/// Extra context-menu action placed above the standard place items.
pub(super) struct PlaceMenuExtra {
    pub(super) icon: &'static str,
    /// English message key, localized when the menu item is built.
    pub(super) label: &'static str,
    pub(super) action: fn(&Rc<SidebarState>),
}

pub(super) fn sidebar_context_option(icon: &str, label: &str, danger: bool) -> gtk::Button {
    let translated = crate::i18n::tr(label);
    let label = translated.as_str();
    let button = crate::ui::accessibility::menu_item_button();
    crate::ui::accessibility::describe_menu_item(&button, label, "");
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = if danger {
        crate::assets::danger_icon(icon, 15)
    } else {
        crate::assets::primary_icon(icon, 15)
    };
    icon.add_css_class("item-context-icon");
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&icon);
    row.append(&title);
    button.set_child(Some(&row));
    button
}
