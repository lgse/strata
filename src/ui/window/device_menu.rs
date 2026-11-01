// SPDX-License-Identifier: MIT

//! The actions menu of a sidebar device row.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{adapters::location_for_file, ui::browser::BrowserView};

use super::{drive_dialogs, drive_ops, sidebar, device_actions::{DeviceRowActions, media_release_label}, drive_dialogs::PropertiesTarget, sidebar_context_menu::sidebar_context_option};

pub(super) fn attach_device_actions_menu(
    row: &gtk::Button,
    view: &BrowserView,
    actions: DeviceRowActions,
    on_crypto: Option<Rc<dyn Fn()>>,
    on_release: Option<Rc<dyn Fn()>>,
    on_mount: Option<Rc<dyn Fn()>>,
    properties: Option<drive_dialogs::PropertiesTarget>,
) {
    let volume = properties
        .as_ref()
        .and_then(drive_dialogs::PropertiesTarget::volume);
    if actions.encrypted.is_none() && actions.release.is_none() && properties.is_none() {
        return;
    }
    let menu = crate::ui::accessibility::menu_box();
    menu.add_css_class("folder-context-menu");
    let popover = gtk::Popover::builder()
        .child(&menu)
        .autohide(true)
        .has_arrow(false)
        .build();
    popover.add_css_class("folder-context-popover");
    popover.set_parent(row);
    let mounted_location = properties
        .as_ref()
        .and_then(|target| match target {
            drive_dialogs::PropertiesTarget::Volume(volume) => volume.get_mount(),
            drive_dialogs::PropertiesTarget::Mount(mount) => Some(mount.clone()),
        })
        .and_then(|mount| location_for_file(&mount.root()));
    if let Some(location) = mounted_location {
        sidebar::append_open_in_tab(&menu, &popover, row, view, &location);
    }
    if let (Some(action), Some(on_crypto)) = (actions.encrypted, on_crypto) {
        let option = sidebar_context_option(action.icon(), action.label(), false);
        menu.append(&option);
        let crypto_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = crypto_popover.upgrade() {
                popover.popdown();
            }
            on_crypto();
        });
    }
    if let (Some(action), Some(on_release)) = (actions.release, on_release.clone()) {
        let option = sidebar_context_option(
            crate::assets::icons::EJECT,
            media_release_label(action),
            false,
        );
        menu.append(&option);
        let release_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = release_popover.upgrade() {
                popover.popdown();
            }
            on_release();
        });
    }
    if drive_ops::show_mount(volume.as_ref())
        && let Some(on_mount) = on_mount
    {
        let option = sidebar_context_option(crate::assets::icons::HARD_DRIVE, "Mount", false);
        menu.append(&option);
        let mount_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = mount_popover.upgrade() {
                popover.popdown();
            }
            on_mount();
        });
    }
    if drive_ops::is_eligible(volume.as_ref())
        && let Some(volume) = &volume
    {
        let format_volume = volume.clone();
        let option = sidebar_context_option(crate::assets::icons::SHREDDER, "Format…", true);
        menu.append(&option);
        let format_popover = popover.downgrade();
        let format_view = view.clone();
        let parent = row.clone().upcast::<gtk::Widget>();
        option.connect_clicked(move |_| {
            drive_dialogs::open_from_sidebar(
                &format_view,
                format_popover.upgrade().as_ref(),
                || {
                    drive_dialogs::show_format_dialog(&parent, &format_volume);
                },
            );
        });
    }
    if let Some(target) = properties
        .as_ref()
        .filter(|target| target.label_id().is_some())
        .cloned()
    {
        let option = sidebar_context_option(crate::assets::icons::PENCIL, "Set label…", false);
        menu.append(&option);
        let label_popover = popover.downgrade();
        let label_view = view.clone();
        let parent = row.clone().upcast::<gtk::Widget>();
        option.connect_clicked(move |_| {
            drive_dialogs::open_from_sidebar(&label_view, label_popover.upgrade().as_ref(), || {
                drive_dialogs::show_label_dialog(&parent, &target);
            });
        });
    }
    if let Some(properties_target) = properties {
        let on_release = on_release.clone();
        let option = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&option);
        let properties_popover = popover.downgrade();
        let properties_view = view.clone();
        let parent = row.clone().upcast::<gtk::Widget>();
        option.connect_clicked(move |_| {
            drive_dialogs::open_from_sidebar(
                &properties_view,
                properties_popover.upgrade().as_ref(),
                || {
                    drive_dialogs::show_drive_properties(
                        &parent,
                        &properties_target,
                        on_release.clone(),
                    );
                },
            );
        });
    }
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
