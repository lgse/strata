// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;

#[derive(Clone)]
pub(super) struct LoadPresentation {
    pub(super) stack: gtk::Stack,
    skeleton: gtk::Box,
    feedback: gtk::Box,
    message: gtk::Label,
    retry: Option<gtk::Button>,
}

impl LoadPresentation {
    pub(super) fn new(content: &impl IsA<gtk::Widget>, retry: Option<gtk::Button>) -> Self {
        let skeleton = crate::ui::loading_skeleton::miller();

        let feedback = gtk::Box::new(gtk::Orientation::Vertical, 8);
        feedback.add_css_class("directory-feedback");
        feedback.set_halign(gtk::Align::Center);
        feedback.set_valign(gtk::Align::Center);
        let message = gtk::Label::new(None);
        message.add_css_class("status-message");
        message.set_justify(gtk::Justification::Center);
        message.set_wrap(true);
        feedback.append(&message);
        if let Some(button) = retry.as_ref() {
            button.set_halign(gtk::Align::Center);
            feedback.append(button);
        }

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(content, Some("content"));
        stack.add_named(&skeleton, Some("loading"));
        stack.add_named(&feedback, Some("feedback"));
        stack.set_visible_child_name("loading");

        Self {
            stack,
            skeleton,
            feedback,
            message,
            retry,
        }
    }

    pub(super) fn show_loading(&self) {
        self.skeleton.set_visible(true);
        self.feedback.set_visible(true);
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.stack.set_visible_child_name("loading");
    }

    pub(super) fn show_content(&self) {
        self.stack.set_visible_child_name("content");
    }

    pub(super) fn show_empty(&self) {
        self.message.set_text("This directory is empty");
        self.message.remove_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.stack.set_visible_child_name("feedback");
    }

    pub(super) fn show_error(&self, message: &str) {
        self.message.set_text(message);
        self.message.add_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(true);
        }
        self.stack.set_visible_child_name("feedback");
    }
}
