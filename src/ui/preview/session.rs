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

    pub(super) fn toggle(self: &Rc<Self>, entry: Option<FileEntry>, depth: Option<usize>) {
        if self.is_enabled() {
            self.close();
        } else {
            self.set_enabled(true);
            if let Some(entry) = entry.and_then(|entry| preview_target(Some(entry))) {
                self.show(entry, depth);
            } else {
                self.clear_target();
            }
        }
    }

    pub(super) fn cancel_selection_task(&self) {
        if let Some(task) = self.selection_task.borrow_mut().take() {
            task.abort();
        }
    }

    pub(super) fn show_selection_summary(
        self: &Rc<Self>,
        entries: Vec<FileEntry>,
        depth: Option<usize>,
    ) {
        self.cancel_selection_task();
        self.selection_summary.set(true);
        self.current_depth.set(depth);
        self.set_enabled(true);

        self.current_request.set(None);
        self.load.borrow_mut().take();
        self.cancel_loading();
        self.pdf_loads.borrow_mut().clear();
        self.clear_content();
        self.current.borrow_mut().take();

        let count = entries.len();
        self.title.set_text(&format!("{count} items selected"));
        self.title.set_tooltip_text(None);
        self.icon.set_visible(false);
        self.open.set_sensitive(false);
        self.open.set_visible(false);
        self.print.set_visible(false);
        self.wrap.set_visible(false);
        self.header_handle.set_cursor_from_name(None);

        self.metadata.set_visible(true);
        self.size.set_text("Calculating…");
        self.size.set_tooltip_text(None);
        self.modified.set_text("—");
        self.content_type.set_text("—");

        let was_open = self.revealer.reveals_child() || self.sizing.is_suspended();
        let split = self.split.borrow().clone();
        if let Some(split) = split.as_ref()
            && (!self.can_show_in(split) || self.sizing.is_suspended())
        {
            self.sizing.defer_load();
        } else if !was_open {
            self.show_panel();
            if let Some(split) = split.as_ref() {
                self.animate_open(split);
            }
        }

        let weak = Rc::downgrade(self);
        let progress_weak = weak.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let summary =
                crate::adapters::directory_summary::summarize_selection(&entries, move |partial| {
                    if let Some(state) = progress_weak.upgrade() {
                        let prefix = if partial.truncated() { "≥ " } else { "" };
                        state
                            .size
                            .set_text(&format!("{prefix}{}", format_file_size(partial.total_size)));
                    }
                })
                .await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            match summary {
                Ok(summary) => {
                    let prefix = if summary.truncated() { "≥ " } else { "" };
                    state
                        .size
                        .set_text(&format!("{prefix}{}", format_file_size(summary.total_size)));
                    state
                        .size
                        .set_tooltip_text(Some(&format!("{} items selected", summary.item_count)));
                }
                Err(_) => state.size.set_text("Unavailable"),
            }
        });
        self.selection_task.replace(Some(task));
    }

    pub(super) fn clear_target(&self) {
        self.cancel_selection_task();
        self.selection_summary.set(false);
        self.animating.set(false);
        self.sizing.close();
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.current_request.set(None);
        self.current_depth.set(None);
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
