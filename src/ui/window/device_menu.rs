// SPDX-License-Identifier: MIT

//! The actions menu of a sidebar device row.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{adapters::location_for_file, ui::browser::BrowserView};

use super::{
    device_actions::{DeviceRowActions, media_release_label},
    drive_dialogs::{self, PropertiesTarget},
    drive_ops, sidebar,
    sidebar_context_menu::sidebar_context_option,
    sidebar_popover::{context_popover, open_on_secondary_click, popdown},
};

/// What a device row's menu can offer, with the handler behind each action.
pub(super) struct DeviceMenuEntries {
    pub(super) actions: DeviceRowActions,
    pub(super) on_crypto: Option<Rc<dyn Fn()>>,
    pub(super) on_release: Option<Rc<dyn Fn()>>,
    pub(super) on_mount: Option<Rc<dyn Fn()>>,
    pub(super) properties: Option<PropertiesTarget>,
}

impl DeviceMenuEntries {
    pub(super) fn offers_actions(&self) -> bool {
        self.actions.encrypted.is_some()
            || self.actions.release.is_some()
            || self.properties.is_some()
    }
}

#[cfg(test)]
pub(super) fn attach_device_actions_menu(
    row: &gtk::Button,
    view: &BrowserView,
    actions: DeviceRowActions,
    on_crypto: Option<Rc<dyn Fn()>>,
    on_release: Option<Rc<dyn Fn()>>,
    on_mount: Option<Rc<dyn Fn()>>,
    properties: Option<PropertiesTarget>,
) {
    attach_device_menu(
        row,
        view,
        DeviceMenuEntries {
            actions,
            on_crypto,
            on_release,
            on_mount,
            properties,
        },
    );
}

pub(super) fn attach_device_menu(
    row: &gtk::Button,
    view: &BrowserView,
    entries: DeviceMenuEntries,
) {
    let volume = entries
        .properties
        .as_ref()
        .and_then(PropertiesTarget::volume);
    if !entries.offers_actions() {
        return;
    }
    let menu = DeviceMenu::new(row, view);
    menu.append_open_in_tab(entries.properties.as_ref());
    if let (Some(action), Some(on_crypto)) = (entries.actions.encrypted, entries.on_crypto) {
        menu.append_action(action.icon(), action.label(), on_crypto);
    }
    if let (Some(action), Some(on_release)) = (entries.actions.release, entries.on_release.clone())
    {
        menu.append_action(
            crate::assets::icons::EJECT,
            media_release_label(action),
            on_release,
        );
    }
    if drive_ops::show_mount(volume.as_ref())
        && let Some(on_mount) = entries.on_mount
    {
        menu.append_action(crate::assets::icons::HARD_DRIVE, "Mount", on_mount);
    }
    if drive_ops::is_eligible(volume.as_ref())
        && let Some(volume) = volume
    {
        menu.append_dialog(
            crate::assets::icons::SHREDDER,
            "Format…",
            true,
            move |parent| {
                drive_dialogs::show_format_dialog(parent, &volume);
            },
        );
    }
    menu.append_dialogs_for(entries.properties, entries.on_release);
    open_on_secondary_click(row, &menu.popover);
}

struct DeviceMenu {
    row: gtk::Button,
    view: BrowserView,
    menu: gtk::Box,
    popover: gtk::Popover,
}

impl DeviceMenu {
    fn new(row: &gtk::Button, view: &BrowserView) -> Self {
        let menu = crate::ui::accessibility::menu_box();
        menu.add_css_class("folder-context-menu");
        let popover = context_popover(row, &menu);
        Self {
            row: row.clone(),
            view: view.clone(),
            menu,
            popover,
        }
    }

    fn append_open_in_tab(&self, properties: Option<&PropertiesTarget>) {
        let mounted_location = properties
            .and_then(|target| match target {
                PropertiesTarget::Volume(volume) => volume.get_mount(),
                PropertiesTarget::Mount(mount) => Some(mount.clone()),
            })
            .and_then(|mount| location_for_file(&mount.root()));
        if let Some(location) = mounted_location {
            sidebar::append_open_in_tab(
                &self.menu,
                &self.popover,
                &self.row,
                &self.view,
                &location,
            );
        }
    }

    /// An item that closes the menu, then runs `action`.
    fn append_action(&self, icon: &str, label: &str, action: Rc<dyn Fn()>) {
        let option = sidebar_context_option(icon, label, false);
        self.menu.append(&option);
        let popover = self.popover.downgrade();
        option.connect_clicked(move |_| {
            popdown(&popover);
            action();
        });
    }

    /// An item that closes the menu and opens a dialog over the row.
    fn append_dialog(
        &self,
        icon: &str,
        label: &str,
        danger: bool,
        show: impl Fn(&gtk::Widget) + 'static,
    ) {
        let option = sidebar_context_option(icon, label, danger);
        self.menu.append(&option);
        let popover = self.popover.downgrade();
        let view = self.view.downgrade();
        // The option sits in a popover under the row, so a strong row or view
        // here would keep both alive after the sidebar is rebuilt or the tab closes.
        let parent = self.row.downgrade();
        option.connect_clicked(move |_| {
            let (Some(view), Some(parent)) = (view.upgrade(), parent.upgrade()) else {
                return;
            };
            drive_dialogs::open_from_sidebar(&view, popover.upgrade().as_ref(), || {
                show(parent.upcast_ref())
            });
        });
    }

    fn append_dialogs_for(
        &self,
        properties: Option<PropertiesTarget>,
        on_release: Option<Rc<dyn Fn()>>,
    ) {
        let Some(properties) = properties else {
            return;
        };
        if properties.label_id().is_some() {
            let target = properties.clone();
            self.append_dialog(
                crate::assets::icons::PENCIL,
                "Set label…",
                false,
                move |parent| {
                    drive_dialogs::show_label_dialog(parent, &target);
                },
            );
        }
        self.append_dialog(
            crate::assets::icons::INFO,
            "Properties",
            false,
            move |parent| {
                drive_dialogs::show_drive_properties(parent, &properties, on_release.clone());
            },
        );
    }
}
