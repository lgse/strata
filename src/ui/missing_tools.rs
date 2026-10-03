// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::{assets, services::package_manager::PackageManager};

use super::{
    controls::{ModalTone, copyable_command, message_dialog_description, message_dialog_layout},
    modal::{ModalHost, dismiss_modal_layer, modal_layer, remember_modal_focus},
};

pub(super) struct MissingTool<'a> {
    pub name: &'a str,
    pub packages: &'a [(PackageManager, &'a str)],
}

fn install_command(manager: Option<PackageManager>, tools: &[MissingTool<'_>]) -> Option<String> {
    let manager = manager?;
    let packages: Option<Vec<_>> = tools
        .iter()
        .map(|tool| {
            tool.packages
                .iter()
                .find_map(|(candidate, package)| (*candidate == manager).then_some(*package))
        })
        .collect();
    manager.install_command(&packages?)
}

pub(super) fn show_missing_tools(
    parent: &impl IsA<gtk::Widget>,
    explanation: &str,
    tools: &[MissingTool<'_>],
) {
    let Some(host) = ModalHost::blurred_for(parent) else {
        return;
    };
    let layout = message_dialog_layout(
        assets::icons::TRIANGLE_ALERT,
        "Missing tools",
        "Install the required tools to continue",
        "Close",
        ModalTone::Accent,
    );
    layout.cancel.set_visible(false);
    layout.body.append(&message_dialog_description(explanation));
    let names = tools
        .iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>()
        .join(", ");
    let missing = message_dialog_description(&format!("Missing: {names}"));
    missing.set_selectable(true);
    layout.body.append(&missing);
    if let Some(command) = install_command(PackageManager::detect(), tools) {
        layout.body.append(&message_dialog_description(
            "Run this command in a terminal, then try the action again:",
        ));
        layout.body.append(&copyable_command(&command));
    } else {
        layout.body.append(&message_dialog_description(
            "Install the packages providing these tools using your system's package manager, then try the action again.",
        ));
    }

    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        None,
    );
    remember_modal_focus(&layer, &host.overlay);
    host.overlay.add_overlay(&layer);
    let dismissed = Cell::new(false);
    let close_layer = layer.clone();
    let close = Rc::new(move || {
        if !dismissed.replace(true) {
            dismiss_modal_layer(&close_layer, &host.overlay, host.blurred_root.as_ref());
        }
    });
    let clicked = close.clone();
    layout.confirm.connect_clicked(move |_| clicked());
    let clicked = close.clone();
    layout.close.connect_clicked(move |_| clicked());
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    layout.confirm.grab_focus();
}

#[cfg(test)]
mod tests;
