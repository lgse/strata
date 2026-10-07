// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::ui::loading_skeleton::{CONTENT_PAGE, LOADING_PAGE};

const FEEDBACK_PAGE: &str = "feedback";

#[derive(Clone)]
pub(super) struct LoadPresentation {
    pub(super) stack: gtk::Stack,
    loading: crate::ui::loading_skeleton::DelayedLoading,
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
            .accessible_role(gtk::AccessibleRole::Group)
            .build();
        stack.add_named(content, Some(CONTENT_PAGE));
        stack.add_named(&skeleton, Some(LOADING_PAGE));
        stack.add_named(&feedback, Some(FEEDBACK_PAGE));
        let loading = crate::ui::loading_skeleton::DelayedLoading::new(&stack);

        Self {
            stack,
            loading,
            message,
            retry,
        }
    }

    /// Lets the stack take keyboard focus in place of `view` while it shows the empty,
    /// error or loading page. Peek columns never call this, so they stay unfocusable.
    pub(super) fn with_focus_fallback(
        self,
        directory: &str,
        view: &impl IsA<gtk::Widget>,
        browser: &Rc<crate::app::Browser>,
    ) -> Self {
        crate::ui::loading_skeleton::DirectorySurface::install(
            &self.stack,
            &self.message,
            view,
            None,
            browser,
            directory,
        );
        self
    }

    pub(super) fn retry_has_focus(&self, focused: &gtk::Widget) -> bool {
        self.retry.as_ref().is_some_and(|retry| {
            crate::ui::focus_navigation::contains_widget(retry.upcast_ref(), Some(focused))
        })
    }

    /// The error page's Retry button is a Tab stop inside the surface.
    pub(super) fn focus_retry(&self) -> bool {
        self.retry
            .as_ref()
            .is_some_and(|retry| retry.is_mapped() && retry.grab_focus())
    }

    pub(super) fn show_loading(&self) {
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.loading.start();
    }

    pub(super) fn show_content(&self) {
        self.loading.show(CONTENT_PAGE);
    }

    pub(super) fn show_empty(&self) {
        self.message
            .set_text(&crate::i18n::tr("This directory is empty"));
        self.message.remove_css_class("error");
        if let Some(retry) = self.retry.as_ref() {
            retry.set_visible(false);
        }
        self.loading.show(FEEDBACK_PAGE);
    }

    pub(super) fn show_empty_if_ready(&self) {
        let showing_error = self.stack.visible_child_name().as_deref() == Some(FEEDBACK_PAGE)
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
        self.loading.show(FEEDBACK_PAGE);
    }
}
