// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::prelude::*;

use crate::ui::loading_skeleton::{CONTENT_PAGE, LOADING_PAGE};

#[cfg(test)]
mod tests;

const FEEDBACK_PAGE: &str = "feedback";

#[derive(Clone)]
pub(super) struct LoadPresentation {
    pub(super) stack: gtk::Stack,
    loading: crate::ui::loading_skeleton::DelayedLoading,
    message: gtk::Label,
    retry: Option<gtk::Button>,
    administrator: Option<gtk::Button>,
}

impl LoadPresentation {
    pub(super) fn new(
        content: &impl IsA<gtk::Widget>,
        retry: Option<gtk::Button>,
        administrator: Option<gtk::Button>,
    ) -> Self {
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
        for button in [administrator.as_ref(), retry.as_ref()]
            .into_iter()
            .flatten()
        {
            button.set_halign(gtk::Align::Center);
            button.set_visible(false);
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
            administrator,
        }
    }

    fn actions(&self) -> impl Iterator<Item = &gtk::Button> {
        [self.administrator.as_ref(), self.retry.as_ref()]
            .into_iter()
            .flatten()
    }

    fn set_actions_visible(&self, retry: bool, administrator: bool) {
        if let Some(button) = self.retry.as_ref() {
            button.set_visible(retry);
        }
        if let Some(button) = self.administrator.as_ref() {
            button.set_visible(administrator);
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

    fn focused_action(&self, focused: &gtk::Widget) -> Option<usize> {
        self.actions().position(|action| {
            crate::ui::focus_navigation::contains_widget(action.upcast_ref(), Some(focused))
        })
    }

    pub(super) fn action_has_focus(&self, focused: &gtk::Widget) -> bool {
        self.focused_action(focused).is_some()
    }

    /// The error page's buttons are Tab stops inside the surface, after it.
    pub(super) fn focus_next_action(&self, focused: &gtk::Widget, surface: &gtk::Widget) -> bool {
        let start = match self.focused_action(focused) {
            Some(index) => index + 1,
            None if focused == surface => 0,
            None => return false,
        };
        self.actions()
            .skip(start)
            .any(|action| action.is_mapped() && action.grab_focus())
    }

    pub(super) fn focus_previous_action(&self, focused: &gtk::Widget) -> bool {
        let Some(index) = self.focused_action(focused) else {
            return false;
        };
        let actions: Vec<_> = self.actions().take(index).collect();
        actions
            .into_iter()
            .rev()
            .any(|action| action.is_mapped() && action.grab_focus())
    }

    pub(super) fn show_loading(&self) {
        self.set_actions_visible(false, false);
        self.loading.start();
    }

    pub(super) fn show_content(&self) {
        self.loading.show(CONTENT_PAGE);
    }

    pub(super) fn show_empty(&self) {
        self.message
            .set_text(&crate::i18n::tr("This directory is empty"));
        self.message.remove_css_class("error");
        self.set_actions_visible(false, false);
        self.loading.show(FEEDBACK_PAGE);
    }

    pub(super) fn show_empty_if_ready(&self) {
        let showing_error = self.stack.visible_child_name().as_deref() == Some(FEEDBACK_PAGE)
            && self.message.has_css_class("error");
        if !showing_error {
            self.show_empty();
        }
    }

    pub(super) fn show_error(&self, message: &str, offers_administrator: bool) {
        self.message.set_text(message);
        self.message.add_css_class("error");
        self.set_actions_visible(true, offers_administrator);
        self.loading.show(FEEDBACK_PAGE);
    }
}

pub(in crate::ui) fn not_responding_notice() -> gtk::Box {
    listing_notice(
        crate::assets::icons::TRIANGLE_ALERT,
        &crate::i18n::tr("Not responding. This list may be out of date."),
    )
}

/// Shown over an `admin://` listing, where every change is made as root.
pub(in crate::ui) fn administrator_notice() -> gtk::Box {
    let notice = listing_notice(
        crate::assets::icons::LOCK,
        &crate::i18n::tr(
            "Browsing as administrator. New items belong to root, and deleted items skip Trash.",
        ),
    );
    notice.set_visible(true);
    notice
}

fn listing_notice(icon: &str, text: &str) -> gtk::Box {
    let notice = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .accessible_role(gtk::AccessibleRole::Status)
        .visible(false)
        .build();
    notice.add_css_class("listing-notice");
    let icon = crate::assets::primary_icon(icon, 16);
    icon.set_valign(gtk::Align::Start);
    notice.append(&icon);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    notice.append(&label);
    notice
}
