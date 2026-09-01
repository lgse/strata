// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;

pub(super) fn form_entry() -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.add_css_class("form-control");
    entry
}

pub(super) fn form_password_entry() -> gtk::PasswordEntry {
    let entry = gtk::PasswordEntry::new();
    entry.add_css_class("form-control");
    entry
}

pub(super) struct ModalLayout {
    pub content: gtk::Box,
    pub body: gtk::Box,
    pub close: gtk::Button,
    pub cancel: gtk::Button,
    pub confirm: gtk::Button,
}

/// Builds the shared structure and styling for an action modal.
pub(super) fn modal_layout(
    icon: &str,
    title: &str,
    subtitle: &str,
    confirm_label: &str,
) -> ModalLayout {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("action-dialog");
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("action-dialog-header");
    header.set_valign(gtk::Align::Center);

    let symbol = gtk::CenterBox::new();
    symbol.add_css_class("action-dialog-symbol");
    symbol.set_size_request(40, 40);
    symbol.set_hexpand(false);
    symbol.set_center_widget(Some(&crate::assets::primary_icon(icon, 21)));

    let heading = gtk::Box::new(gtk::Orientation::Vertical, 1);
    heading.add_css_class("action-dialog-heading");
    heading.set_hexpand(true);
    heading.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(title));
    title.add_css_class("action-dialog-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some(subtitle));
    subtitle.add_css_class("action-dialog-subtitle");
    subtitle.set_xalign(0.0);
    heading.append(&title);
    heading.append(&subtitle);

    let close = gtk::Button::new();
    close.add_css_class("action-dialog-close");
    close.set_valign(gtk::Align::Center);
    close.set_tooltip_text(Some("Cancel"));
    close.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::X,
        16,
    )));

    header.append(&symbol);
    header.append(&heading);
    header.append(&close);
    content.append(&header);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.add_css_class("action-dialog-body");
    content.append(&body);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("action-dialog-actions");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("action-dialog-cancel");
    let confirm = gtk::Button::with_label(confirm_label);
    confirm.add_css_class("action-dialog-confirm");
    actions.append(&spacer);
    actions.append(&cancel);
    actions.append(&confirm);
    content.append(&actions);

    ModalLayout {
        content,
        body,
        close,
        cancel,
        confirm,
    }
}

/// Builds a single-selection group with the same compact treatment as a segmented control.
pub(super) fn segmented_control(
    labels: &[&str],
    selected: usize,
) -> (gtk::Box, Vec<gtk::ToggleButton>) {
    let control = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    control.set_homogeneous(true);
    control.add_css_class("segmented-control");

    let mut buttons = Vec::with_capacity(labels.len());
    for (index, label) in labels.iter().enumerate() {
        let button = gtk::ToggleButton::with_label(label);
        button.add_css_class("segmented-control-option");
        button.set_hexpand(true);
        if index == 0 {
            button.add_css_class("first");
        } else {
            button.add_css_class("not-first");
        }
        if index + 1 == labels.len() {
            button.add_css_class("last");
        }
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        button.set_active(index == selected);
        control.append(&button);
        buttons.push(button);
    }

    (control, buttons)
}
