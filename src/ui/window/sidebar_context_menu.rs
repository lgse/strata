// SPDX-License-Identifier: MIT

//! The context menu attached to sidebar places.

use std::{path::PathBuf, rc::Rc};

use gtk::prelude::*;

use crate::{model::Location, ui::browser::BrowserView};

use super::{
    SidebarState, sidebar,
    sidebar_popover::{context_popover, open_on_secondary_click, popdown},
};

impl SidebarState {
    pub(super) fn attach_place_context_menu(
        self: &Rc<Self>,
        row: &gtk::Button,
        location: Location,
        on_unpin: impl Fn(&Rc<Self>) + 'static,
    ) {
        let place = PlaceMenu {
            location,
            extra: None,
        };
        self.attach_place_context_menu_with(row, place, on_unpin);
    }

    /// Like [`Self::attach_place_context_menu`], with the place's extra item above the
    /// standard items.
    pub(super) fn attach_place_context_menu_with(
        self: &Rc<Self>,
        row: &gtk::Button,
        place: PlaceMenu,
        on_unpin: impl Fn(&Rc<Self>) + 'static,
    ) {
        let menu = crate::ui::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let extra_item = place.extra.map(|extra| {
            let button = sidebar_context_option(extra.icon, extra.label, false);
            menu.append(&button);
            (button, extra.action)
        });
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&unpin);
        let customize = place.location.native_path().map(|path| {
            let button = sidebar_context_option(crate::assets::icons::PALETTE, "Customize…", false);
            menu.append(&button);
            (button, path.to_path_buf())
        });
        menu.append(&properties);
        let popover = context_popover(row, &menu);
        sidebar::append_open_in_tab(&menu, &popover, row, &self.view(), &place.location);
        if let Some((button, action)) = extra_item {
            self.connect_state_item(&button, &popover, action);
        }
        if let Some((customize, path)) = customize {
            connect_customize(&customize, &popover, row, path);
        }
        self.connect_state_item(&unpin, &popover, on_unpin);
        connect_properties(&properties, &popover, &self.view(), place.location);
        open_on_secondary_click(row, &popover);
    }

    /// Closes the menu, then runs `action` while the sidebar is still alive.
    pub(super) fn connect_state_item(
        self: &Rc<Self>,
        button: &gtk::Button,
        popover: &gtk::Popover,
        action: impl Fn(&Rc<Self>) + 'static,
    ) {
        let popover = popover.downgrade();
        let weak_state = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            popdown(&popover);
            if let Some(state) = weak_state.upgrade() {
                action(&state);
            }
        });
    }
}

fn connect_customize(
    customize: &gtk::Button,
    popover: &gtk::Popover,
    row: &gtk::Button,
    path: PathBuf,
) {
    let weak_popover = popover.downgrade();
    let weak_row = row.downgrade();
    customize.connect_clicked(move |_| {
        popdown(&weak_popover);
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

fn connect_properties(
    properties: &gtk::Button,
    popover: &gtk::Popover,
    view: &BrowserView,
    location: Location,
) {
    let properties_view = view.downgrade();
    let properties_popover = popover.downgrade();
    properties.connect_clicked(move |_| {
        popdown(&properties_popover);
        if let Some(view) = properties_view.upgrade() {
            view.show_location_properties(&location);
        }
    });
}

/// The place a context menu acts on, and the extra item it offers.
pub(super) struct PlaceMenu {
    pub(super) location: Location,
    pub(super) extra: Option<PlaceMenuExtra>,
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
