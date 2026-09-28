// SPDX-License-Identifier: MIT

use gtk::prelude::*;

#[derive(Clone)]
pub(super) struct LoadPresentation {
    pub(super) stack: gtk::Stack,
    loading: crate::ui::loading_skeleton::DelayedLoading,
    message: gtk::Label,
    retry: Option<gtk::Button>,
    empty_text: std::rc::Rc<std::cell::RefCell<String>>,
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
        let loading = crate::ui::loading_skeleton::DelayedLoading::new(&stack);

        Self {
            stack,
            loading,
            message,
            retry,
            empty_text: std::rc::Rc::new(std::cell::RefCell::new("This directory is empty".into())),
        }
    }

    /// Some locations explain an empty listing, such as undiscovered networks.
    pub(super) fn set_empty_text(&self, text: String) {
        self.empty_text.replace(text);
    }

    pub(super) fn show_loading(&self) {
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.loading.start();
    }

    pub(super) fn show_content(&self) {
        self.loading.show("content");
    }

    pub(super) fn show_empty(&self) {
        self.message.set_text(&self.empty_text.borrow());
        self.message.remove_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.loading.show("feedback");
    }

    pub(super) fn show_empty_if_ready(&self) {
        let showing_error = self.stack.visible_child_name().as_deref() == Some("feedback")
            && self.message.has_css_class("error");
        if !showing_error {
            self.show_empty();
        }
    }

    pub(super) fn show_error(&self, message: &str) {
        self.message.set_text(message);
        self.message.add_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(true);
        }
        self.loading.show("feedback");
    }
}
