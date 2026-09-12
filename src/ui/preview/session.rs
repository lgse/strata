// SPDX-License-Identifier: MIT

use super::*;

impl PreviewDrawer {
    pub fn is_enabled(&self) -> bool {
        self.state.is_enabled()
    }

    pub(in crate::ui) fn action(&self) -> gio::SimpleAction {
        self.state.enabled_action.clone()
    }

    pub(in crate::ui) fn clear_target(&self) {
        self.state.clear_target();
    }
}

impl PreviewState {
    pub(super) fn is_enabled(&self) -> bool {
        self.enabled_action
            .state()
            .and_then(|value| value.get::<bool>())
            .unwrap_or(false)
    }

    pub(super) fn set_enabled(&self, enabled: bool) -> bool {
        let previous = self.is_enabled();
        if previous != enabled {
            self.enabled_action.set_state(&enabled.to_variant());
        }
        previous
    }

    pub(super) fn toggle(self: &Rc<Self>, entry: Option<FileEntry>) {
        if self.is_enabled() {
            self.close();
        } else {
            self.set_enabled(true);
            if let Some(entry) = entry.and_then(|entry| preview_target(Some(entry))) {
                self.show(entry);
            } else {
                self.clear_target();
            }
        }
    }

    pub(super) fn clear_target(&self) {
        self.animating.set(false);
        self.sizing.close();
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.current_request.set(None);
        self.current.borrow_mut().take();
        self.load.borrow_mut().take();
        self.cancel_loading();
        self.pdf_loads.borrow_mut().clear();
        self.clear_content();
        if self.reserves_empty_preview() {
            self.show_placeholder();
        } else {
            self.hide_panel();
        }
    }

    pub(super) fn show_placeholder(&self) {
        if self
            .content
            .first_child()
            .is_some_and(|child| child.has_css_class("preview-placeholder"))
        {
            return;
        }
        self.clear_content();
        self.title.set_text(PREVIEW_LABEL);
        self.title.set_tooltip_text(None);
        self.icon.set_visible(false);
        self.metadata.set_visible(false);
        self.open.set_sensitive(false);
        self.header_handle.set_cursor_from_name(None);
        let placeholder = gtk::Label::builder()
            .label("No preview for this selection")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .hexpand(true)
            .vexpand(true)
            .margin_start(24)
            .margin_end(24)
            .build();
        placeholder.add_css_class("preview-placeholder");
        placeholder.add_css_class("preview-feedback-detail");
        self.content.append(&placeholder);
    }
}
