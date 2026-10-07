// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::prelude::*;

pub(super) fn stepper(labels: [&str; 3]) -> (gtk::Box, [gtk::Button; 3]) {
    let control = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    control.add_css_class("appearance-text-stepper");
    control.set_hexpand(true);
    control.set_halign(gtk::Align::End);
    let buttons = std::array::from_fn(|index| {
        let label = crate::i18n::tr(labels[index]);
        let button = gtk::Button::new();
        if index == 1 {
            button.add_css_class("appearance-text-value");
            button.set_hexpand(true);
            super::accessibility::set_description(&button, Some(&label));
        } else {
            let icon = if index == 0 {
                crate::assets::icons::MINUS
            } else {
                crate::assets::icons::PLUS
            };
            let image = crate::assets::primary_icon(icon, 16);
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            button.set_child(Some(&image));
            button.add_css_class("appearance-text-step");
            super::accessibility::set_label(&button, &label);
            button.set_tooltip_text(Some(&label));
        }
        control.append(&button);
        button
    });
    (control, buttons)
}

pub(super) fn pane_header_action(widget: &impl IsA<gtk::Widget>) {
    widget.add_css_class("column-header-action");
    widget.set_valign(gtk::Align::Center);
    widget.set_cursor_from_name(Some("pointer"));
}

pub(super) fn form_entry() -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.add_css_class("form-control");
    entry
}

pub(super) struct FormTextField {
    pub widget: gtk::Box,
    pub entry: gtk::Entry,
    remaining: gtk::Label,
}

impl FormTextField {
    pub fn with_character_limit(max_length: i32) -> Self {
        let entry = form_entry();
        entry.set_max_length(max_length);
        let remaining = form_label("");
        remaining.set_halign(gtk::Align::End);
        remaining.set_xalign(1.0);
        super::accessibility::set_label(&remaining, &crate::i18n::tr("Characters remaining"));

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        widget.append(&entry);
        widget.append(&remaining);
        let field = Self {
            widget,
            entry,
            remaining,
        };
        field.refresh_remaining();
        let remaining = field.remaining.clone();
        field.entry.connect_changed(move |entry| {
            Self::update_remaining(entry, &remaining);
        });
        let remaining = field.remaining.clone();
        field.entry.connect_max_length_notify(move |entry| {
            Self::update_remaining(entry, &remaining);
        });
        field
    }

    fn refresh_remaining(&self) {
        Self::update_remaining(&self.entry, &self.remaining);
    }

    fn update_remaining(entry: &gtk::Entry, remaining: &gtk::Label) {
        let limit = entry.max_length();
        remaining.set_visible(limit > 0);
        let count = (limit as usize).saturating_sub(entry.text().chars().count());
        remaining.set_text(&format!("{count} chars remaining"));
    }
}

#[cfg(test)]
mod tests;

pub(super) fn copyable_command(command: &str) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("preview-command");
    overlay.set_hexpand(true);

    let field = form_entry();
    field.add_css_class("preview-command-entry");
    field.set_text(command);
    field.set_editable(false);
    field.set_hexpand(true);
    overlay.set_child(Some(&field));

    let copy = gtk::Button::builder()
        .tooltip_text(crate::i18n::tr("Copy install command"))
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();
    copy.add_css_class("preview-command-copy");
    copy.set_has_frame(false);
    copy.set_cursor_from_name(Some("pointer"));
    let copy_icon = crate::assets::primary_icon(crate::assets::icons::COPY, 16);
    copy.set_child(Some(&copy_icon));
    let copied_command = command.to_owned();
    let feedback_generation = Rc::new(Cell::new(0_u64));
    copy.connect_clicked(move |button| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&copied_command);
        }
        let generation = feedback_generation.get().saturating_add(1);
        feedback_generation.set(generation);
        crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::CHECK);
        button.set_tooltip_text(Some(&crate::i18n::tr("Install command copied")));
        let button = button.clone();
        let copy_icon = copy_icon.clone();
        let feedback_generation = feedback_generation.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if feedback_generation.get() == generation {
                crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::COPY);
                button.set_tooltip_text(Some(&crate::i18n::tr("Copy install command")));
            }
        });
    });
    overlay.add_overlay(&copy);
    overlay
}

pub(super) struct ProgressSummary {
    pub widget: gtk::Box,
    pub header: gtk::Box,
    pub amount: gtk::Label,
    pub percent: gtk::Label,
    pub progress: gtk::ProgressBar,
}

pub(super) fn progress_summary(caption: &str) -> ProgressSummary {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("transfer-progress-header");
    let amount_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    amount_box.set_hexpand(true);
    let caption = gtk::Label::new(Some(&crate::i18n::tr(caption)));
    caption.add_css_class("transfer-progress-caption");
    caption.set_xalign(0.0);
    let amount = gtk::Label::new(None);
    amount.add_css_class("transfer-progress-bytes");
    amount.set_xalign(0.0);
    amount_box.append(&caption);
    amount_box.append(&amount);
    let percent = gtk::Label::new(None);
    percent.add_css_class("transfer-progress-percent");
    header.append(&amount_box);
    header.append(&percent);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("modal-progress");
    progress.set_fraction(0.0);
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 12);
    widget.append(&header);
    widget.append(&progress);
    ProgressSummary {
        widget,
        header,
        amount,
        percent,
        progress,
    }
}

pub(super) fn properties_action(icon: &str, label: &str, tone: ModalTone) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.set_halign(gtk::Align::Center);
    let image = match tone {
        ModalTone::Accent => crate::assets::primary_icon(icon, 14),
        ModalTone::Danger => crate::assets::danger_icon(icon, 14),
    };
    content.append(&image);
    content.append(&gtk::Label::new(Some(&crate::i18n::tr(label))));
    let button = gtk::Button::builder().child(&content).build();
    match tone {
        ModalTone::Accent => button.add_css_class("properties-action"),
        ModalTone::Danger => {
            button.add_css_class("action-dialog-confirm");
            button.add_css_class("danger");
        }
    }
    button.set_hexpand(true);
    button
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

pub(super) fn form_error_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("form-field-error");
    label.set_xalign(0.0);
    label.set_visible(false);
    label
}

pub(super) fn set_form_field_error(
    field: &impl IsA<gtk::Widget>,
    helper: &gtk::Label,
    message: Option<&str>,
) {
    if let Some(message) = message {
        field.add_css_class("error");
        helper.set_text(&crate::i18n::tr(message));
        helper.set_visible(true);
    } else {
        field.remove_css_class("error");
        helper.set_visible(false);
    }
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
    pub icon: gtk::Image,
}

pub(super) fn focus_button(button: &gtk::Button) {
    let weak = button.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(button) = weak.upgrade() {
            button.grab_focus();
            if let Some(window) = button.root().and_downcast::<gtk::Window>() {
                window.set_focus_visible(false);
            }
        }
    });
}

impl ModalLayout {
    pub fn set_loading(&self, loading: bool, description: Option<&str>) {
        if loading {
            crate::ui::accessibility::set_description(
                &self.loading,
                description.or(Some("Working…")),
            );
            self.loading.set_visible(true);
            self.loading.start();
        } else {
            self.loading.stop();
            self.loading.set_visible(false);
            crate::ui::accessibility::set_description(&self.loading, None);
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
    let content = super::accessibility::dialog_box(title);
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
    symbol.set_hexpand(false);
    symbol.set_valign(gtk::Align::Fill);
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
    close.set_tooltip_text(Some(&crate::i18n::tr("Close dialog")));
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
    let cancel = gtk::Button::with_label(&crate::i18n::tr("Cancel"));
    cancel.add_css_class("action-dialog-cancel");
    let confirm = gtk::Button::with_label(&crate::i18n::tr(confirm_label));
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
        icon,
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
        let button = gtk::ToggleButton::with_label(&crate::i18n::tr(label));
        button.add_css_class("segmented-control-option");
        button.set_hexpand(true);
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        button.set_active(index == selected);
        control.append(&button);
        buttons.push(button);
    }

    (control, buttons)
}
