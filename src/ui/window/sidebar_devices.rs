// SPDX-License-Identifier: MIT

//! Volume, mount and drive rows in the sidebar Devices section.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use crate::{adapters::location_for_file, model::Location, ui::browser::show_error_dialog};

use super::{
    SidebarState,
    device_actions::{
        DeviceRowActions, EncryptedMediaAction, MediaRelease, device_row_actions,
        media_release_label, release_device_mount, release_device_volume,
    },
    device_labels,
    device_lock::{eject_password_drive, lock_encrypted_mount, lock_encrypted_volume},
    device_menu::attach_device_actions_menu,
    device_release, drive_dialogs,
    gio_devices::{
        gio_mount_is_encrypted, gio_volume_is_encrypted, gio_volume_is_locked,
        gio_volume_unix_device, orphaned_password_drives,
    },
    launch::navigate_to_gio_file,
    sidebar,
    sidebar::PlaceNavigation,
    sidebar_context_menu::sidebar_context_option,
    sidebar_file_drop::install_sidebar_file_drop,
    sidebar_rows::{select_sidebar_row, sidebar_button},
    sidebar_sections::SidebarSection,
};

impl SidebarState {
    pub(super) fn append_devices(self: &Rc<Self>) {
        let volumes = self.volume_monitor.volumes();
        let mounts = self.mounts_without_volumes(&volumes);
        let password_drives =
            orphaned_password_drives(&volumes, &self.volume_monitor.connected_drives());
        if volumes.is_empty()
            && mounts.is_empty()
            && password_drives.is_empty()
            && !device_release::any_pending()
        {
            return;
        }
        if self.widget.first_child().is_some() {
            self.append_separator();
        }
        let section = self.append_devices_section();
        let mut shown = Vec::new();
        for volume in volumes {
            if let Some(ids) = self.append_volume(volume) {
                shown.push(ids);
            }
        }
        for drive in password_drives {
            if let Some(ids) = self.append_password_drive(drive) {
                shown.push(ids);
            }
        }
        for (name, location, mount) in mounts {
            if let Some(ids) = self.append_mount(&name, location, mount) {
                shown.push(ids);
            }
        }
        for key in device_release::unmatched_pending(&shown) {
            self.append_release_ghost(&key);
        }
        let count = section.collect_rows(&self.widget);
        section.header.set_item_count(count);
        section.apply_state(self.section_collapsed(SidebarSection::Devices));
        self.devices_section.replace(Some(section));
    }

    pub(super) fn mounts_without_volumes(
        &self,
        volumes: &[gio::Volume],
    ) -> Vec<(String, Location, gio::Mount)> {
        self.volume_monitor
            .mounts()
            .into_iter()
            .filter(|mount| !mount.is_shadowed())
            .filter(|mount| {
                !mount
                    .volume()
                    .is_some_and(|volume| volumes.contains(&volume))
            })
            .filter_map(|mount| {
                let name = mount.name().to_string();
                let location = location_for_file(&mount.root())?;
                if self.local_only && location.native_path().is_none() {
                    return None;
                }
                Some((name, location, mount))
            })
            .collect()
    }

    pub(super) fn append_mount(
        self: &Rc<Self>,
        name: &str,
        location: Location,
        mount: gio::Mount,
    ) -> Option<device_release::DeviceIds> {
        if is_smb_location(&location) {
            self.append_smb_mount(name, location, mount);
            return None;
        }
        let ids = device_release::ids_for_mount(&mount);
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, name);
        if let Some(id) = drive_dialogs::PropertiesTarget::Mount(mount.clone()).label_id() {
            device_labels::bind_row_label(&row, &self.preference_manager, &id, name);
        }
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        if device_release::is_pending(&ids) {
            self.append_sidebar_row(&device_release::pending_device_shell(&row));
            return Some(ids);
        }
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        let encrypted = gio_mount_is_encrypted(&mount);
        let actions = device_row_actions(
            encrypted,
            true,
            false,
            mount.can_eject(),
            mount.can_unmount(),
        );
        let in_flight = Rc::new(Cell::new(false));
        let on_crypto = actions.encrypted.map(|_| {
            let crypto_mount = mount.clone();
            let crypto_parent = self.view.widget();
            let crypto_browser = self.browser.clone();
            let crypto_view = self.view.clone();
            let crypto_in_flight = in_flight.clone();
            Rc::new(move || {
                lock_encrypted_mount(
                    &crypto_mount,
                    &crypto_parent,
                    &crypto_browser,
                    &crypto_view,
                    &crypto_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        let on_release = actions.release.map(|action| {
            let release_mount = mount.clone();
            let release_browser = self.browser.clone();
            let release_parent = self.view.widget();
            let release_view = self.view.clone();
            let release_in_flight = in_flight.clone();
            Rc::new(move || {
                release_device_mount(
                    &release_mount,
                    action,
                    &release_parent,
                    &release_browser,
                    &release_view,
                    &release_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        self.append_device_chrome(
            &row,
            actions,
            on_crypto,
            on_release,
            None,
            Some(drive_dialogs::PropertiesTarget::Mount(mount)),
        );
        Some(ids)
    }

    pub(super) fn append_volume(
        self: &Rc<Self>,
        volume: gio::Volume,
    ) -> Option<device_release::DeviceIds> {
        let name = volume.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        if let Some(id) = drive_dialogs::PropertiesTarget::Volume(volume.clone()).label_id() {
            device_labels::bind_row_label(&row, &self.preference_manager, &id, &name);
        }
        crate::ui::accessibility::set_description(&row, Some(&name));
        let mounted_location = volume
            .get_mount()
            .as_ref()
            .and_then(|mount| location_for_file(&mount.root()));
        if let Some(location) = mounted_location.as_ref() {
            if self.local_only && location.native_path().is_none() {
                return None;
            }
        } else if self.local_only
            && (gio_volume_unix_device(&volume).is_none()
                || volume
                    .activation_root()
                    .is_some_and(|root| root.path().is_none()))
        {
            return None;
        }
        let ids = device_release::ids_for_volume(&volume);
        if device_release::is_pending(&ids) {
            self.append_sidebar_row(&device_release::pending_device_shell(&row));
            return Some(ids);
        }
        if let Some(location) = mounted_location {
            sidebar::install_open_in_tab(&row, &self.view, &location, &self.drop_feedback);
            self.place_rows
                .borrow_mut()
                .push((location.clone(), row.clone()));
            install_sidebar_file_drop(self, &row, location);
        }
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let clicked_volume = volume.clone();
        let mount_view = self.view.clone();
        row.connect_clicked(move |_| {
            let volume = clicked_volume.clone();
            select_sidebar_row(&sidebar, &selected_row);
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(mount) = volume.get_mount() {
                navigate_to_gio_file(&browser, &mount.root());
                return;
            }

            mount_view.mount_volume(volume);
        });
        let (mount_can_eject, mount_can_unmount) = volume
            .get_mount()
            .map(|mount| (mount.can_eject(), mount.can_unmount()))
            .unwrap_or((false, false));
        let encrypted = gio_volume_is_encrypted(&volume);
        let mounted = !gio_volume_is_locked(&volume);
        let actions = device_row_actions(
            encrypted,
            mounted,
            volume.can_eject(),
            mount_can_eject,
            mount_can_unmount,
        );
        let in_flight = Rc::new(Cell::new(false));
        let on_release = actions.release.map(|action| {
            let release_volume = volume.clone();
            let release_browser = self.browser.clone();
            let release_parent = self.view.widget();
            let release_view = self.view.clone();
            let release_in_flight = in_flight.clone();
            Rc::new(move || {
                release_device_volume(
                    &release_volume,
                    action,
                    &release_parent,
                    &release_browser,
                    &release_view,
                    &release_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        let on_crypto = actions.encrypted.map(|action| {
            let crypto_volume = volume.clone();
            let crypto_view = self.view.clone();
            let crypto_parent = self.view.widget();
            let crypto_browser = self.browser.clone();
            let crypto_in_flight = in_flight.clone();
            Rc::new(move || match action {
                EncryptedMediaAction::Unlock => crypto_view.unlock_volume(crypto_volume.clone()),
                EncryptedMediaAction::Lock => lock_encrypted_volume(
                    &crypto_volume,
                    &crypto_parent,
                    &crypto_browser,
                    &crypto_view,
                    &crypto_in_flight,
                ),
            }) as Rc<dyn Fn()>
        });
        let on_mount = {
            let mount_view = self.view.clone();
            let mount_volume = volume.clone();
            Rc::new(move || mount_view.mount_volume(mount_volume.clone())) as Rc<dyn Fn()>
        };
        self.append_device_chrome(
            &row,
            actions,
            on_crypto,
            on_release,
            Some(on_mount),
            Some(drive_dialogs::PropertiesTarget::Volume(volume)),
        );
        Some(ids)
    }

    pub(super) fn append_release_ghost(&self, key: &device_release::ReleaseKey) {
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &key.display_name);
        self.append_sidebar_row(&device_release::pending_device_shell(&row));
    }

    pub(super) fn append_password_drive(
        &self,
        drive: gio::Drive,
    ) -> Option<device_release::DeviceIds> {
        let ids = device_release::ids_for_drive(&drive);
        let name = drive.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        crate::ui::accessibility::set_description(&row, Some(&name));
        if device_release::is_pending(&ids) {
            self.append_sidebar_row(&device_release::pending_device_shell(&row));
            return Some(ids);
        }
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let clicked_drive = drive.clone();
        let mount_view = self.view.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            mount_view.start_password_drive(clicked_drive.clone(), true);
        });
        let actions = device_row_actions(true, false, drive.can_eject(), false, false);
        let on_crypto = Some({
            let drive = drive.clone();
            let view = self.view.clone();
            Rc::new(move || view.start_password_drive(drive.clone(), true)) as Rc<dyn Fn()>
        });
        let on_release = actions.release.map(|action| {
            let release_drive = drive.clone();
            let release_parent = self.view.widget();
            let release_view = self.view.clone();
            let release_in_flight = Rc::new(Cell::new(false));
            Rc::new(move || {
                eject_password_drive(
                    &release_drive,
                    action,
                    &release_parent,
                    &release_view,
                    &release_in_flight,
                );
            }) as Rc<dyn Fn()>
        });
        self.append_device_chrome(&row, actions, on_crypto, on_release, None, None);
        Some(ids)
    }

    pub(super) fn append_smb_mount(
        self: &Rc<Self>,
        name: &str,
        location: Location,
        mount: gio::Mount,
    ) {
        let properties_location = location.clone();
        let row = self.append_place(crate::assets::icons::NETWORK, name, location);
        let menu = crate::ui::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let disconnect = sidebar_context_option(crate::assets::icons::UNPLUG, "Disconnect", true);
        disconnect.add_css_class("danger");
        menu.append(&properties);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&disconnect);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);
        sidebar::append_open_in_tab(&menu, &popover, &row, &self.view, &properties_location);

        let properties_popover = popover.downgrade();
        let properties_view = self.view.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            if let Some(view) = properties_view.upgrade() {
                view.show_location_properties(&properties_location);
            }
        });

        let disconnect_popover = popover.downgrade();
        let parent = self.view.widget();
        disconnect.connect_clicked(move |_| {
            if let Some(popover) = disconnect_popover.upgrade() {
                popover.popdown();
            }
            let window = parent.root().and_downcast::<gtk::Window>();
            let operation = gtk::MountOperation::new(window.as_ref());
            let mount = mount.clone();
            let error_parent = parent.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(error) = mount
                    .unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation))
                    .await
                    && !error.matches(gio::IOErrorEnum::Cancelled)
                {
                    show_error_dialog(
                        &error_parent,
                        &crate::i18n::tr("Unable to disconnect"),
                        &error.to_string(),
                    );
                }
            });
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

    pub(super) fn append_device_place(
        &self,
        icon: &str,
        name: &str,
        location: Location,
        release: Option<(MediaRelease, Rc<dyn Fn()>)>,
    ) -> gtk::Button {
        let row = sidebar_button(icon, name);
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        match release {
            Some((action, on_release)) => {
                let eject = sidebar_eject_button(action, {
                    let on_release = on_release.clone();
                    move || on_release()
                });
                attach_device_actions_menu(
                    &row,
                    &self.view,
                    DeviceRowActions {
                        encrypted: None,
                        release: Some(action),
                    },
                    None,
                    Some(on_release),
                    None,
                    None,
                );
                self.append_sidebar_row(&sidebar_device_row(&row, None, Some(&eject)));
            }
            None => {
                self.append_sidebar_row(&row);
            }
        }
        row
    }

    pub(super) fn append_device_chrome(
        &self,
        row: &gtk::Button,
        actions: DeviceRowActions,
        on_crypto: Option<Rc<dyn Fn()>>,
        on_release: Option<Rc<dyn Fn()>>,
        on_mount: Option<Rc<dyn Fn()>>,
        properties: Option<drive_dialogs::PropertiesTarget>,
    ) {
        let lock = actions
            .encrypted
            .zip(on_crypto.clone())
            .map(|(action, on_crypto)| sidebar_lock_button(action, move || on_crypto()));
        let eject = actions
            .release
            .zip(on_release.clone())
            .map(|(action, on_release)| sidebar_eject_button(action, move || on_release()));
        if actions.encrypted.is_some() || actions.release.is_some() || properties.is_some() {
            attach_device_actions_menu(
                row, &self.view, actions, on_crypto, on_release, on_mount, properties,
            );
            self.append_sidebar_row(&sidebar_device_row(row, lock.as_ref(), eject.as_ref()));
        } else {
            self.append_sidebar_row(row);
        }
    }
}

pub(super) fn sidebar_device_shell(row: &gtk::Button) -> Option<gtk::Box> {
    row.parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
        .filter(|parent| parent.has_css_class("sidebar-device"))
}

pub(super) fn is_smb_location(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        uri.get(..4)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("smb:"))
    })
}

pub(super) fn sidebar_device_row(
    row: &gtk::Button,
    lock: Option<&gtk::Button>,
    eject: Option<&gtk::Button>,
) -> gtk::Box {
    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.add_css_class("sidebar-device");
    shell.set_hexpand(true);
    row.set_hexpand(true);
    row.set_halign(gtk::Align::Fill);
    row.connect_has_focus_notify(|row| {
        let Some(shell) = sidebar_device_shell(row) else {
            return;
        };
        if row.has_focus() {
            shell.add_css_class("focused");
        } else {
            shell.remove_css_class("focused");
        }
    });
    shell.append(row);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("sidebar-device-actions");
    actions.set_hexpand(false);
    actions.set_halign(gtk::Align::End);
    actions.set_valign(gtk::Align::Center);
    if let Some(lock) = lock {
        lock.set_hexpand(false);
        actions.append(lock);
    }
    if let Some(eject) = eject {
        eject.set_hexpand(false);
        actions.append(eject);
    }
    if actions.first_child().is_some() {
        shell.append(&actions);
    }
    shell
}

pub(super) fn sidebar_eject_button(
    action: MediaRelease,
    on_release: impl Fn() + 'static,
) -> gtk::Button {
    let button = gtk::Button::new();
    crate::ui::accessibility::set_tooltip_and_label(
        &button,
        &crate::i18n::tr(media_release_label(action)),
    );
    button.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::EJECT,
        14,
    )));
    button.add_css_class("sidebar-eject");
    button.add_css_class("sidebar-device-action");
    button.set_cursor_from_name(Some("pointer"));
    button.set_has_frame(false);
    button.set_hexpand(false);
    button.set_valign(gtk::Align::Center);
    button.set_width_request(24);
    button.connect_clicked(move |_| on_release());
    button
}

pub(super) fn sidebar_lock_button(
    action: EncryptedMediaAction,
    on_click: impl Fn() + 'static,
) -> gtk::Button {
    let button = gtk::Button::new();
    crate::ui::accessibility::set_tooltip_and_label(&button, &crate::i18n::tr(action.label()));
    button.set_child(Some(&crate::assets::primary_icon(action.icon(), 14)));
    button.add_css_class("sidebar-eject");
    button.add_css_class("sidebar-device-action");
    button.set_cursor_from_name(Some("pointer"));
    button.set_has_frame(false);
    button.set_hexpand(false);
    button.set_valign(gtk::Align::Center);
    button.set_width_request(24);
    button.connect_clicked(move |_| on_click());
    button
}
