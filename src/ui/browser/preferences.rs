// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::preferences::PreferenceManager;

impl BrowserView {
    fn bind_view_preference<T: PartialEq + Clone + 'static>(
        &self,
        manager: &PreferenceManager,
        read: impl Fn(&PreferenceManager) -> T + 'static,
        apply: impl Fn(&Self, T) + 'static,
    ) {
        let weak = Rc::downgrade(&self.state);
        manager.bind_preference(&self.widget(), read, move |_, value| {
            if let Some(state) = weak.upgrade() {
                apply(&Self { state }, value);
            }
        });
    }

    pub(super) fn bind_preferences(&self, manager: &PreferenceManager) {
        self.bind_view_preference(
            manager,
            PreferenceManager::thumbnail_workers,
            |_, workers| {
                super::super::thumbnail::set_worker_limit(workers);
            },
        );
        // The folder is part of the value, so a size read before navigating
        // cannot hide a later change.
        let weak = Rc::downgrade(&self.state);
        self.bind_view_preference(
            manager,
            move |manager| {
                let location = weak.upgrade().and_then(|state| {
                    let browser = &state.browser;
                    browser.location_at(browser.active_depth()?)
                });
                let size = manager.icons_size_for(location.as_ref());
                (location, size)
            },
            |view, (_, size)| view.apply_resolved_icons_size(size),
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::browser_mode,
            Self::set_view_mode,
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::browser_density,
            Self::set_density,
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::group_by_type,
            Self::set_group_by_type,
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::auto_refresh_interval,
            Self::set_auto_refresh_interval,
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::single_click_previews,
            Self::set_single_click_previews,
        );
        let primed = Cell::new(false);
        self.bind_view_preference(
            manager,
            PreferenceManager::tenxer_mode,
            move |view, enabled| {
                view.browser().set_preserve_fill_on_removal(enabled);
                if primed.replace(true) && !enabled {
                    view.end_tenxer_session();
                }
            },
        );
        let interactive = self.state.interactive;
        self.bind_view_preference(
            manager,
            move |manager| interactive && manager.columns_mirror_selection(),
            Self::set_columns_mirror_selection,
        );
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
        self.bind_view_preference(
            manager,
            PreferenceManager::sort_preferences,
            |view, value| {
                view.browser().apply_default_preferences(value);
            },
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::remember_folder_views,
            |view, remember| {
                view.browser().set_folder_sorts(remember.then(|| {
                    Rc::new(|location: &crate::model::Location, opened| {
                        PreferenceManager::shared().resolve_folder_sort(location, opened)
                    }) as crate::app::FolderSortResolver
                }));
            },
        );
        self.bind_view_preference(
            manager,
            PreferenceManager::folder_sorts_revision,
            |view, _| view.browser().resync_column_sorts(),
        );
    }
}
