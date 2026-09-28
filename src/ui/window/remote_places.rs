// SPDX-License-Identifier: MIT

//! Sidebar rows for saved Connections and transient network mounts.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    services::{connections::SavedConnection, remote::RemoteProtocol},
    ui::connections::{
        ConnectionEditor, confirm_remove_connection, connection_target, connection_tooltip,
        disconnect_mount, matching_mount, mount_is_saved, show_connection_editor,
        show_rename_connection,
    },
};

use super::{
    SidebarState, is_context_menu_shortcut, sidebar::PlaceNavigation, sidebar_button,
    sidebar_context_option, sidebar_device_row,
};

pub(super) struct RowAction {
    icon: &'static str,
    label: &'static str,
    danger: bool,
    run: Rc<dyn Fn()>,
}

impl RowAction {
    pub(super) fn new(icon: &'static str, label: &'static str, run: impl Fn() + 'static) -> Self {
        Self {
            icon,
            label,
            danger: false,
            run: Rc::new(run),
        }
    }

    fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// A row context menu opened by a secondary click or the keyboard's
/// context-menu shortcuts (Menu, Shift+F10).
pub(super) fn attach_row_menu(row: &gtk::Button, actions: Vec<RowAction>) {
    let menu = crate::ui::accessibility::menu_box();
    menu.add_css_class("folder-context-menu");
    let popover = gtk::Popover::builder()
        .child(&menu)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");
    popover.set_parent(row);
    for action in actions {
        let option = sidebar_context_option(action.icon, action.label, action.danger);
        if action.danger {
            option.add_css_class("danger");
        }
        let weak_popover = popover.downgrade();
        let run = action.run;
        option.connect_clicked(move |_| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
            run();
        });
        menu.append(&option);
    }

    let context = gtk::GestureClick::new();
    context.set_button(3);
    let weak_popover = popover.downgrade();
    context.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(popover) = weak_popover.upgrade() {
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        }
    });
    row.add_controller(context);

    let keys = gtk::EventControllerKey::new();
    let weak_popover = popover.downgrade();
    keys.connect_key_pressed(move |controller, key, _, modifiers| {
        if !is_context_menu_shortcut(key, modifiers) {
            return glib::Propagation::Proceed;
        }
        let (Some(popover), Some(row)) = (weak_popover.upgrade(), controller.widget()) else {
            return glib::Propagation::Proceed;
        };
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            row.width() / 2,
            row.height() / 2,
            1,
            1,
        )));
        popover.popup();
        if let Some(first) = popover.child().and_then(|menu| menu.first_child()) {
            first.grab_focus();
        }
        glib::Propagation::Stop
    });
    row.add_controller(keys);
}

fn disconnect_button(on_click: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text("Disconnect").build();
    crate::ui::accessibility::set_label(&button, "Disconnect");
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::UNPLUG,
        14,
    )));
    button.add_css_class("sidebar-device-action");
    button.set_cursor_from_name(Some("pointer"));
    button.set_has_frame(false);
    button.set_hexpand(false);
    button.set_valign(gtk::Align::Center);
    button.set_width_request(24);
    button.connect_clicked(move |_| on_click());
    button
}

impl SidebarState {
    fn disconnect_action(self: &Rc<Self>, mount: &gio::Mount) -> impl Fn() + 'static {
        let mount = mount.clone();
        let parent = self.view.widget();
        let weak = Rc::downgrade(self);
        move || {
            let weak = weak.clone();
            disconnect_mount(&parent, mount.clone(), move || {
                if let Some(state) = weak.upgrade() {
                    state.queue_rebuild();
                }
            });
        }
    }

    pub(super) fn add_connection_action(&self) -> impl Fn() + 'static {
        let parent = self.view.widget();
        move || show_connection_editor(&parent, ConnectionEditor::Add(None))
    }

    /// Saved connections, distinct from Pinned folders and transient Devices.
    pub(super) fn append_connections(self: &Rc<Self>) {
        if self.local_only {
            return;
        }
        let connections = crate::ui::connections::saved_connections().connections();
        self.saved_connections.replace(connections.clone());
        if connections.is_empty() {
            return;
        }
        if self.widget.first_child().is_some() {
            self.append_separator();
        }
        self.append_connections_heading();
        for connection in connections {
            self.append_connection(connection);
        }
    }

    fn append_connections_heading(&self) {
        let heading_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        heading_row.add_css_class("sidebar-heading-row");
        let heading = gtk::Label::new(Some("CONNECTIONS"));
        heading.add_css_class("sidebar-heading");
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        let add = gtk::Button::builder()
            .tooltip_text("Add connection…")
            .build();
        crate::ui::accessibility::set_label(&add, "Add connection");
        add.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::PLUS,
            14,
        )));
        add.add_css_class("sidebar-device-action");
        add.add_css_class("sidebar-heading-action");
        add.set_cursor_from_name(Some("pointer"));
        add.set_has_frame(false);
        add.set_valign(gtk::Align::Center);
        let open_editor = self.add_connection_action();
        add.connect_clicked(move |_| open_editor());
        heading_row.append(&heading);
        heading_row.append(&add);
        self.widget.append(&heading_row);
    }

    fn append_connection(self: &Rc<Self>, connection: SavedConnection) {
        let mount = matching_mount(&connection).map(|(mount, _)| mount);
        let connected = mount.is_some();
        let row = sidebar_button(
            if connected {
                crate::assets::icons::SERVER
            } else {
                crate::assets::icons::SERVER_OFF
            },
            &connection.name,
        );
        row.add_css_class("sidebar-connection-row");
        if !connected {
            row.add_css_class("offline");
        }
        let tooltip = connection_tooltip(&connection, connected);
        row.set_tooltip_text(Some(&tooltip));
        row.update_property(&[gtk::accessible::Property::Description(&tooltip)]);
        self.row_tooltips.borrow_mut().push((row.clone(), tooltip));
        self.bind_place_row(
            &row,
            connection_target(&connection),
            PlaceNavigation::Validate,
        );

        let parent = self.view.widget();
        let mut actions = vec![
            RowAction::new(crate::assets::icons::PENCIL, "Rename…", {
                let parent = parent.clone();
                let connection = connection.clone();
                move || show_rename_connection(&parent, connection.clone())
            }),
            RowAction::new(crate::assets::icons::SETTINGS, "Edit…", {
                let parent = parent.clone();
                let connection = connection.clone();
                move || show_connection_editor(&parent, ConnectionEditor::Edit(connection.clone()))
            }),
        ];
        let removable = mount.filter(gio::Mount::can_unmount);
        if let Some(mount) = &removable {
            actions.push(RowAction::new(
                crate::assets::icons::UNPLUG,
                "Disconnect",
                self.disconnect_action(mount),
            ));
        }
        actions.push(
            RowAction::new(crate::assets::icons::TRASH, "Remove…", {
                let connection = connection.clone();
                move || confirm_remove_connection(&parent, connection.clone())
            })
            .danger(),
        );
        attach_row_menu(&row, actions);
        match removable {
            Some(mount) => {
                let disconnect = disconnect_button(self.disconnect_action(&mount));
                self.widget
                    .append(&sidebar_device_row(&row, None, Some(&disconnect)));
            }
            None => self.widget.append(&row),
        }
    }

    /// Transient network mounts of any protocol. Disconnect is offered only
    /// when the mount reports it can be unmounted.
    pub(super) fn append_network_mount(
        self: &Rc<Self>,
        name: &str,
        location: Location,
        mount: gio::Mount,
    ) {
        let row = sidebar_button(crate::assets::icons::NETWORK, name);
        row.set_tooltip_text(Some(&location.display_path()));
        self.bind_place_row(&row, location.clone(), PlaceNavigation::Validate);
        let parent = self.view.widget();
        let mut actions = vec![RowAction::new(crate::assets::icons::INFO, "Properties", {
            let view = self.view.clone();
            let location = location.clone();
            move || view.show_location_properties(&location)
        })];
        if let Some(form) = crate::services::connections::ConnectionForm::from_location(&location) {
            actions.push(RowAction::new(
                crate::assets::icons::SERVER,
                "Save Connection…",
                move || show_connection_editor(&parent, ConnectionEditor::Add(Some(form.clone()))),
            ));
        }
        let removable = mount.can_unmount().then_some(mount);
        if let Some(mount) = &removable {
            actions.push(
                RowAction::new(
                    crate::assets::icons::UNPLUG,
                    "Disconnect",
                    self.disconnect_action(mount),
                )
                .danger(),
            );
        }
        attach_row_menu(&row, actions);
        match removable {
            Some(mount) => {
                let disconnect = disconnect_button(self.disconnect_action(&mount));
                self.widget
                    .append(&sidebar_device_row(&row, None, Some(&disconnect)));
            }
            None => self.widget.append(&row),
        }
    }

    /// A mount already shown as a saved connection isn't repeated in Devices.
    pub(super) fn mount_has_connection_row(&self, mount: &gio::Mount) -> bool {
        !self.local_only && mount_is_saved(&self.saved_connections.borrow(), mount)
    }
}

pub(super) fn is_network_location(location: &Location) -> bool {
    RemoteProtocol::for_location(location).is_some()
}
