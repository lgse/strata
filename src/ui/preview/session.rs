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
        self.hide_panel();
    }
}
