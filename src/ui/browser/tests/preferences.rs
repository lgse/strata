// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

impl BrowserView {
    pub(in crate::ui) fn assert_saved_preferences(&self, manager: &crate::ui::theme::ThemeManager) {
        assert_eq!(self.state.peek_enabled.get(), manager.folder_peeking());
        assert_eq!(
            self.single_click_previews_enabled(),
            manager.single_click_previews()
        );
        assert_eq!(
            self.state.columns_click_activation.get(),
            manager.click_activation(BrowserMode::Columns)
        );
        assert_eq!(
            self.state.auto_refresh.borrow().is_some(),
            manager.auto_refresh_interval() != 0
        );
        assert_eq!(self.browser().preferences(), manager.sort_preferences());
        self.state
            .mode_views
            .borrow()
            .assert_saved_preferences(manager);
    }

    pub(in crate::ui) fn assert_peek_scheduling(&self, enabled: bool) {
        self.state.input_ownership.borrow_mut().last_navigation =
            crate::ui::input_ownership::NavigationInput::Pointer;
        self.state.schedule_peek(
            0,
            Location::local("/fixture/child"),
            gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
        );
        assert_eq!(self.state.pending_peek.borrow().is_some(), enabled);
        cancel_source(&self.state.pending_peek);
    }
}
