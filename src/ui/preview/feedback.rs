// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn show_message(&self, title: &str, detail: &str) {
        self.show_message_with_icon(title, detail, None, None);
    }

    pub(super) fn show_message_with_external_open(&self, title: &str, detail: &str) {
        let feedback = self.show_message_with_icon(title, detail, None, None);
        let open = gtk::Button::with_label(&crate::i18n::tr("Open"));
        open.add_css_class("suggested-action");
        open.add_css_class("preview-feedback-action");
        open.set_halign(gtk::Align::Center);
        let header_open = self.open.clone();
        open.connect_clicked(move |_| header_open.emit_clicked());
        feedback.append(&open);
        self.hand_keys_to(&open);
    }

    pub(super) fn show_message_with_icon(
        &self,
        title: &str,
        detail: &str,
        icon: Option<&str>,
        command: Option<&str>,
    ) -> gtk::Box {
        self.clear_content();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 7);
        box_.add_css_class("preview-feedback");
        box_.set_halign(gtk::Align::Center);
        box_.set_valign(gtk::Align::Center);
        box_.set_vexpand(true);
        if let Some(icon) = icon {
            let icon = crate::assets::primary_icon(icon, 34);
            icon.add_css_class("preview-feedback-icon");
            icon.set_halign(gtk::Align::Center);
            box_.append(&icon);
        }
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("preview-feedback-title");
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("preview-feedback-detail");
        detail.set_justify(gtk::Justification::Center);
        detail.set_wrap(true);
        box_.append(&heading);
        box_.append(&detail);
        if let Some(command) = command {
            box_.append(&super::super::controls::copyable_command(command));
        }
        self.content.append(&box_);
        box_
    }
}
