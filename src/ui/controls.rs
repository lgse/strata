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

pub(super) fn form_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("action-dialog-field-label");
    label.set_xalign(0.0);
    label
}

pub(super) fn form_check_button(label: &str) -> gtk::CheckButton {
    let button = gtk::CheckButton::with_label(label);
    button.add_css_class("form-check");
    button
}

pub(super) fn menu_option(label: &str, selected: bool) -> (gtk::Button, gtk::Image) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let check = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
    check.set_visible(selected);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    row.append(&check);
    let option = gtk::Button::builder().child(&row).build();
    option.add_css_class("column-menu-option");
    option.set_has_frame(false);
    (option, check)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ModalTone {
    #[default]
    Accent,
    Danger,
}

pub(super) const MESSAGE_DIALOG_WIDTH_CHARS: usize = 64;

pub(super) fn wrap_dialog_text(text: &str, max_chars: usize) -> String {
    let mut wrapped = String::new();
    let mut line_chars = 0;
    for word in text.split_whitespace() {
        let chunks = word
            .chars()
            .collect::<Vec<_>>()
            .chunks(max_chars.max(1))
            .map(|chunk| chunk.iter().collect::<String>())
            .collect::<Vec<_>>();
        for (index, chunk) in chunks.iter().enumerate() {
            let chunk_chars = chunk.chars().count();
            if line_chars > 0 && line_chars + 1 + chunk_chars > max_chars {
                wrapped.push('\n');
                line_chars = 0;
            } else if line_chars > 0 {
                wrapped.push(' ');
                line_chars += 1;
            }
            wrapped.push_str(chunk);
            line_chars += chunk_chars;
            if index + 1 < chunks.len() {
                wrapped.push('\n');
                line_chars = 0;
            }
        }
    }
    wrapped
}

pub(super) fn message_dialog_description(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&wrap_dialog_text(text, MESSAGE_DIALOG_WIDTH_CHARS)));
    label.add_css_class("action-dialog-description");
    label.set_max_width_chars(MESSAGE_DIALOG_WIDTH_CHARS as i32);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label
}

pub(super) struct ModalLayout {
    pub content: gtk::Box,
    pub body: gtk::Box,
    pub actions: gtk::Box,
    pub title: gtk::Label,
    pub subtitle: gtk::Label,
    pub loading: gtk::Spinner,
    pub close: gtk::Button,
    pub cancel: gtk::Button,
    pub confirm: gtk::Button,
}

impl ModalLayout {
    pub fn set_loading(&self, loading: bool, tooltip: Option<&str>) {
        if loading {
            self.loading.set_tooltip_text(tooltip.or(Some("Working…")));
            self.loading.set_visible(true);
            self.loading.start();
        } else {
            self.loading.stop();
            self.loading.set_visible(false);
            self.loading.set_tooltip_text(None);
        }
    }
}

/// Builds the shared structure and styling for an action modal.
pub(super) fn modal_layout(
    icon: &str,
    title: &str,
    subtitle: &str,
    confirm_label: &str,
) -> ModalLayout {
    modal_layout_with_tone(icon, title, subtitle, confirm_label, ModalTone::Accent)
}

pub(super) fn message_dialog_layout(
    icon: &str,
    title: &str,
    subtitle: &str,
    confirm_label: &str,
    tone: ModalTone,
) -> ModalLayout {
    let layout = modal_layout_with_tone(
        icon,
        &wrap_dialog_text(title, MESSAGE_DIALOG_WIDTH_CHARS),
        &wrap_dialog_text(subtitle, MESSAGE_DIALOG_WIDTH_CHARS),
        confirm_label,
        tone,
    );
    layout.content.add_css_class("message-dialog");
    layout.content.set_size_request(560, -1);
    for label in [&layout.title, &layout.subtitle] {
        label.set_max_width_chars(MESSAGE_DIALOG_WIDTH_CHARS as i32);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    }
    layout
}

pub(super) fn modal_layout_with_tone(
    icon: &str,
    title: &str,
    subtitle: &str,
    confirm_label: &str,
    tone: ModalTone,
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
    if tone == ModalTone::Danger {
        symbol.add_css_class("danger");
    }
    symbol.set_size_request(40, 40);
    symbol.set_hexpand(false);
    let icon = match tone {
        ModalTone::Accent => crate::assets::primary_icon(icon, 21),
        ModalTone::Danger => crate::assets::danger_icon(icon, 21),
    };
    symbol.set_center_widget(Some(&icon));

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

    let loading = gtk::Spinner::new();
    loading.add_css_class("action-dialog-loading");
    loading.set_visible(false);

    let close = gtk::Button::new();
    close.add_css_class("action-dialog-close");
    close.set_valign(gtk::Align::Center);
    close.set_tooltip_text(Some("Close dialog"));
    close.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::X,
        16,
    )));

    header.append(&symbol);
    header.append(&heading);
    header.append(&loading);
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
    if tone == ModalTone::Danger {
        confirm.add_css_class("danger");
    }
    actions.append(&spacer);
    actions.append(&cancel);
    actions.append(&confirm);
    content.append(&actions);

    ModalLayout {
        content,
        body,
        actions,
        title,
        subtitle,
        loading,
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
