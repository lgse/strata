// SPDX-License-Identifier: MIT

//! Volume, mount and drive rows in the sidebar Devices section.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use crate::{adapters::location_for_file, model::Location};

use super::{
    SidebarState,
    device_actions::{
        DeviceActionContext, DeviceRowActions, EncryptedMediaAction, MediaRelease,
        device_row_actions, media_release_label, release_device_mount, release_device_volume,
    },
    device_labels,
    device_lock::{eject_password_drive, lock_encrypted_mount, lock_encrypted_volume},
    device_menu::{DeviceMenuEntries, attach_device_menu},
    device_release,
    drive_dialogs::PropertiesTarget,
    gio_devices::{
        gio_mount_is_encrypted, gio_volume_is_encrypted, gio_volume_is_locked,
        gio_volume_unix_device, orphaned_password_drives,
    },
    launch::navigate_to_gio_file,
    sidebar,
    sidebar::PlaceNavigation,
    sidebar_context_menu::sidebar_context_option,
    sidebar_file_drop::install_sidebar_file_drop,
    sidebar_popover::{context_popover, open_on_secondary_click, popdown},
    sidebar_rows::{select_sidebar_row, sidebar_button},
    sidebar_sections::SidebarSection,
};

type DeviceAction = Rc<dyn Fn()>;

impl SidebarState {
    pub(super) fn append_devices(self: &Rc<Self>) {
        let volumes = self.volume_monitor.volumes();
        let mounts = self.mounts_without_volumes(&volumes);
        let password_drives =
            orphaned_password_drives(&volumes, &self.volume_monitor.connected_drives());
        let listed = volumes.len() + mounts.len() + password_drives.len();
        if listed == 0 && !device_release::any_pending() {
            return;
        }
        if self.widget.first_child().is_some() {
            self.append_separator();
        }
        let section = self.append_devices_section();
        let mut shown = Vec::new();
        shown.extend(
            volumes
                .into_iter()
                .filter_map(|volume| self.append_volume(volume)),
        );
        shown.extend(
            password_drives
                .into_iter()
                .filter_map(|drive| self.append_password_drive(drive)),
        );
        shown.extend(
            mounts
                .into_iter()
                .filter_map(|(name, location, mount)| self.append_mount(&name, location, mount)),
        );
        for key in device_release::unmatched_pending(&shown) {
            self.append_release_ghost(&key);
        }
        let count = section.collect_rows(&self.widget);
        section.header.set_item_count(count);
        section.apply_state(
            self.section_collapsed(SidebarSection::Devices),
            self.rail.get(),
        );
        self.devices_section.replace(Some(section));
    }

    pub(super) fn mounts_without_volumes(
        &self,
        volumes: &[gio::Volume],
    ) -> Vec<(String, Location, gio::Mount)> {
        super::devices::mounts_without_listed_volumes(&self.volume_monitor, volumes)
            .into_iter()
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

    /// Lock and release actions on one row share a context, so only one runs at a time.
    fn device_action_context(&self) -> DeviceActionContext {
        DeviceActionContext {
            parent: self.view.widget(),
            browser: Some(self.browser.clone()),
            view: self.view.clone(),
            in_flight: Rc::new(Cell::new(false)),
        }
    }

    /// Shows a pending stand-in instead of the row while the device is being released.
    fn append_if_releasing(&self, row: &gtk::Button, ids: &device_release::DeviceIds) -> bool {
        let releasing = device_release::is_pending(ids);
        if releasing {
            self.append_sidebar_row(&device_release::pending_device_shell(row));
        }
        releasing
    }

    pub(super) fn append_mount(
        self: &Rc<Self>,
        name: &str,
        location: Location,
        mount: gio::Mount,
    ) -> Option<device_release::DeviceIds> {
        if is_network_share(&location) {
            self.append_network_share(name, location, mount);
            return None;
        }
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, name);
        if let Some(id) = PropertiesTarget::Mount(mount.clone()).label_id() {
            device_labels::bind_row_label(&row, &self.preference_manager, &id, name);
        }
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        let ids = device_release::ids_for_mount(&mount);
        if self.append_if_releasing(&row, &ids) {
            return Some(ids);
        }
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        let entries = self.mount_menu_entries(mount);
        self.append_device_chrome(&row, entries);
        Some(ids)
    }

    fn mount_menu_entries(&self, mount: gio::Mount) -> DeviceMenuEntries {
        let actions = device_row_actions(
            gio_mount_is_encrypted(&mount),
            true,
            false,
            mount.can_eject(),
            mount.can_unmount(),
        );
        let context = self.device_action_context();
        let on_crypto = actions.encrypted.map(|_| {
            let mount = mount.clone();
            let context = context.clone();
            Rc::new(move || lock_encrypted_mount(&mount, &context)) as DeviceAction
        });
        let on_release = actions.release.map(|action| {
            let mount = mount.clone();
            let context = context.clone();
            Rc::new(move || release_device_mount(&mount, action, &context)) as DeviceAction
        });
        DeviceMenuEntries {
            actions,
            on_crypto,
            on_release,
            on_mount: None,
            properties: Some(PropertiesTarget::Mount(mount)),
        }
    }

    pub(super) fn append_volume(
        self: &Rc<Self>,
        volume: gio::Volume,
    ) -> Option<device_release::DeviceIds> {
        let name = volume.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        if let Some(id) = PropertiesTarget::Volume(volume.clone()).label_id() {
            device_labels::bind_row_label(&row, &self.preference_manager, &id, &name);
        }
        let mounted_location = volume
            .get_mount()
            .as_ref()
            .and_then(|mount| location_for_file(&mount.root()));
        crate::ui::accessibility::set_description(
            &row,
            volume_description(&volume, mounted_location.as_ref()).as_deref(),
        );
        if self.hides_volume(&volume, mounted_location.as_ref()) {
            return None;
        }
        let ids = device_release::ids_for_volume(&volume);
        if self.append_if_releasing(&row, &ids) {
            return Some(ids);
        }
        if let Some(location) = mounted_location {
            self.install_mounted_volume_row(&row, location);
        }
        self.connect_volume_click(&row, &volume);
        let entries = self.volume_menu_entries(volume);
        self.append_device_chrome(&row, entries);
        Some(ids)
    }

    /// A local-only sidebar lists only volumes that are, or would mount as, local paths.
    fn hides_volume(&self, volume: &gio::Volume, mounted_location: Option<&Location>) -> bool {
        if !self.local_only {
            return false;
        }
        match mounted_location {
            Some(location) => location.native_path().is_none(),
            None => {
                gio_volume_unix_device(volume).is_none()
                    || volume
                        .activation_root()
                        .is_some_and(|root| root.path().is_none())
            }
        }
    }

    fn install_mounted_volume_row(self: &Rc<Self>, row: &gtk::Button, location: Location) {
        sidebar::install_open_in_tab(row, &self.view, &location, &self.drop_feedback);
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        install_sidebar_file_drop(self, row, location);
    }

    fn connect_volume_click(&self, row: &gtk::Button, volume: &gio::Volume) {
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.downgrade();
        let clicked_volume = volume.clone();
        let mount_view = self.view.downgrade();
        row.connect_clicked(move |row| {
            let volume = clicked_volume.clone();
            if let Some(sidebar) = sidebar.upgrade() {
                select_sidebar_row(&sidebar, row);
            }
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(mount) = volume.get_mount() {
                navigate_to_gio_file(&browser, &mount.root());
                return;
            }
            if let Some(view) = mount_view.upgrade() {
                view.mount_volume(volume);
            }
        });
    }

    fn volume_menu_entries(&self, volume: gio::Volume) -> DeviceMenuEntries {
        let (mount_can_eject, mount_can_unmount) = volume
            .get_mount()
            .map(|mount| (mount.can_eject(), mount.can_unmount()))
            .unwrap_or((false, false));
        let actions = device_row_actions(
            gio_volume_is_encrypted(&volume),
            !gio_volume_is_locked(&volume),
            volume.can_eject(),
            mount_can_eject,
            mount_can_unmount,
        );
        let context = self.device_action_context();
        let on_release = actions.release.map(|action| {
            let volume = volume.clone();
            let context = context.clone();
            Rc::new(move || release_device_volume(&volume, action, &context)) as DeviceAction
        });
        let on_crypto = actions.encrypted.map(|action| {
            let volume = volume.clone();
            let context = context.clone();
            Rc::new(move || match action {
                EncryptedMediaAction::Unlock => context.view.unlock_volume(volume.clone()),
                EncryptedMediaAction::Lock => lock_encrypted_volume(&volume, &context),
            }) as DeviceAction
        });
        let on_mount = {
            let mount_view = self.view.clone();
            let mount_volume = volume.clone();
            Rc::new(move || mount_view.mount_volume(mount_volume.clone())) as DeviceAction
        };
        DeviceMenuEntries {
            actions,
            on_crypto,
            on_release,
            on_mount: Some(on_mount),
            properties: Some(PropertiesTarget::Volume(volume)),
        }
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
        if self.append_if_releasing(&row, &ids) {
            return Some(ids);
        }
        let sidebar = self.widget.downgrade();
        let clicked_drive = drive.clone();
        let mount_view = self.view.downgrade();
        row.connect_clicked(move |row| {
            if let Some(sidebar) = sidebar.upgrade() {
                select_sidebar_row(&sidebar, row);
            }
            if let Some(view) = mount_view.upgrade() {
                view.start_password_drive(clicked_drive.clone(), true);
            }
        });
        let entries = self.password_drive_menu_entries(drive);
        self.append_device_chrome(&row, entries);
        Some(ids)
    }

    fn password_drive_menu_entries(&self, drive: gio::Drive) -> DeviceMenuEntries {
        let actions = device_row_actions(true, false, drive.can_eject(), false, false);
        let on_crypto = Some({
            let drive = drive.clone();
            let view = self.view.clone();
            Rc::new(move || view.start_password_drive(drive.clone(), true)) as DeviceAction
        });
        let on_release = actions.release.map(|action| {
            let context = DeviceActionContext {
                parent: self.view.widget(),
                browser: None,
                view: self.view.clone(),
                in_flight: Rc::new(Cell::new(false)),
            };
            Rc::new(move || eject_password_drive(&drive, action, &context)) as DeviceAction
        });
        DeviceMenuEntries {
            actions,
            on_crypto,
            on_release,
            on_mount: None,
            properties: None,
        }
    }

    pub(super) fn append_network_share(
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
        let popover = context_popover(&row, &menu);
        sidebar::append_open_in_tab(&menu, &popover, &row, &self.view, &properties_location);

        let properties_popover = popover.downgrade();
        let properties_view = self.view.downgrade();
        properties.connect_clicked(move |_| {
            popdown(&properties_popover);
            if let Some(view) = properties_view.upgrade() {
                view.show_location_properties(&properties_location);
            }
        });

        let disconnect_popover = popover.downgrade();
        let parent = self.view.widget();
        let browser = Rc::downgrade(&self.browser);
        let in_flight = Rc::new(Cell::new(false));
        disconnect.connect_clicked(move |_| {
            popdown(&disconnect_popover);
            device_release::disconnect_share(
                &parent,
                browser.upgrade(),
                mount.clone(),
                Rc::clone(&in_flight),
            );
        });
        open_on_secondary_click(&row, &popover);
    }

    pub(super) fn append_device_place(
        &self,
        icon: &str,
        name: &str,
        location: Location,
        release: Option<(MediaRelease, DeviceAction)>,
    ) -> gtk::Button {
        let row = sidebar_button(icon, name);
        crate::ui::accessibility::set_description(&row, Some(&location.display_path()));
        self.bind_place_row(&row, location, PlaceNavigation::Validate);
        let Some((action, on_release)) = release else {
            self.append_sidebar_row(&row);
            return row;
        };
        let eject = sidebar_eject_button(action, {
            let on_release = on_release.clone();
            move || on_release()
        });
        let entries = DeviceMenuEntries {
            actions: DeviceRowActions {
                encrypted: None,
                release: Some(action),
            },
            on_crypto: None,
            on_release: Some(on_release),
            on_mount: None,
            properties: None,
        };
        attach_device_menu(&row, &self.view, entries);
        self.append_sidebar_row(&sidebar_device_row(&row, None, Some(&eject)));
        row
    }

    pub(super) fn append_device_chrome(&self, row: &gtk::Button, entries: DeviceMenuEntries) {
        let lock = entries
            .actions
            .encrypted
            .zip(entries.on_crypto.clone())
            .map(|(action, on_crypto)| sidebar_lock_button(action, move || on_crypto()));
        let eject = entries
            .actions
            .release
            .zip(entries.on_release.clone())
            .map(|(action, on_release)| sidebar_eject_button(action, move || on_release()));
        if entries.offers_actions() {
            attach_device_menu(row, &self.view, entries);
            self.append_sidebar_row(&sidebar_device_row(row, lock.as_ref(), eject.as_ref()));
        } else {
            self.append_sidebar_row(row);
        }
    }
}

/// Like a mount row, a volume row is described by its path: where it is mounted,
/// else its device.
fn volume_description(volume: &gio::Volume, mounted: Option<&Location>) -> Option<String> {
    mounted
        .map(Location::display_path)
        .or_else(|| gio_volume_unix_device(volume).map(String::from))
}

pub(super) fn sidebar_device_shell(row: &gtk::Button) -> Option<gtk::Box> {
    row.parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
        .filter(|parent| parent.has_css_class("sidebar-device"))
}

/// GVfs network backends: their mounts are shares to disconnect, not drives to eject.
const NETWORK_SHARE_SCHEMES: &[&str] = &["smb", "sftp", "ftp", "ftps", "dav", "davs", "afp", "nfs"];

pub(super) fn is_network_share(location: &Location) -> bool {
    location.uri_value().is_some_and(|uri| {
        gtk::glib::Uri::parse_scheme(uri).is_some_and(|scheme| {
            NETWORK_SHARE_SCHEMES
                .iter()
                .any(|known| scheme.eq_ignore_ascii_case(known))
        })
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
    for button in [lock, eject].into_iter().flatten() {
        button.set_hexpand(false);
        actions.append(button);
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
    device_action_button(
        &crate::i18n::tr(media_release_label(action)),
        crate::assets::icons::EJECT,
        on_release,
    )
}

pub(super) fn sidebar_lock_button(
    action: EncryptedMediaAction,
    on_click: impl Fn() + 'static,
) -> gtk::Button {
    device_action_button(&crate::i18n::tr(action.label()), action.icon(), on_click)
}

fn device_action_button(label: &str, icon: &str, on_click: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::new();
    crate::ui::accessibility::set_tooltip_and_label(&button, label);
    let image = crate::assets::primary_icon(icon, 14);
    image.set_halign(gtk::Align::End);
    image.set_valign(gtk::Align::Center);
    button.set_child(Some(&image));
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
