// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::theme::ThemeManager;

impl BrowserView {
    fn bind_view_preference<T: PartialEq + Clone + 'static>(
        &self,
        manager: &ThemeManager,
        read: impl Fn(&ThemeManager) -> T + 'static,
        apply: impl Fn(&Self, T) + 'static,
    ) {
        let weak = Rc::downgrade(&self.state);
        manager.bind_preference(&self.widget(), read, move |_, value| {
            if let Some(state) = weak.upgrade() {
                apply(&Self { state }, value);
            }
        });
    }

    pub(super) fn bind_preferences(&self, manager: &ThemeManager) {
        self.bind_view_preference(manager, ThemeManager::browser_mode, Self::set_view_mode);
        self.bind_view_preference(manager, ThemeManager::browser_density, Self::set_density);
        self.bind_view_preference(
            manager,
            ThemeManager::group_by_type,
            Self::set_group_by_type,
        );
        self.bind_view_preference(
            manager,
            ThemeManager::auto_refresh_interval,
            Self::set_auto_refresh_interval,
        );
        self.bind_view_preference(
            manager,
            ThemeManager::single_click_previews,
            Self::set_single_click_previews,
        );
        let interactive = self.state.interactive;
        self.bind_view_preference(
            manager,
            move |manager| interactive && manager.folder_peeking(),
            Self::set_peek_enabled,
        );
        for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
            self.bind_view_preference(
                manager,
                move |manager| manager.click_activation(mode),
                move |view, value| view.set_click_activation(mode, value),
            );
        }
        self.bind_view_preference(manager, ThemeManager::sort_preferences, |view, value| {
            view.browser().apply_default_preferences(value);
        });
    }
}
