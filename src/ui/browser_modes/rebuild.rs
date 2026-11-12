// SPDX-License-Identifier: MIT

//! Switching presentations, and building, reusing, or clearing the pane each shows.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use super::{
    BrowserMode, ClickActivation, ModeViews,
    icons_pane::{
        IconsLayout, IconsOptions, build_icons_pane, compact_icon_size, configure_icons_density,
    },
    list_pane::build_list_pane,
    pane::{
        ModeClickOptions, Pane, PaneBuild, apply_snapshot, clear_box, deactivate_pane_models,
        detach_pane_models, install_pane_surface,
    },
};
use crate::{app::BrowserColumnSnapshot, ui::preferences::PreferenceManager};

impl ModeViews {
    pub fn cancel_pending_restore(&mut self) {
        self.cursor_focus_generation
            .set(self.cursor_focus_generation.get().wrapping_add(1));
        self.pane_navigation.borrow_mut().cancel();
    }

    pub fn prepare_mode(&mut self, mode: BrowserMode) {
        if self.mode == mode {
            return;
        }
        self.cancel_rename();
        self.cancel_pending_restore();
        self.mode = mode;
        self.sync_type_grouping();
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons | BrowserMode::List | BrowserMode::Compact => {
                self.prepare_pane(mode)
            }
        }
    }

    /// Reuses the mode's pane when it still shows the active folder as it should,
    /// and rebuilds it otherwise.
    fn prepare_pane(&mut self, mode: BrowserMode) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_mode(mode);
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if let Some(pane) = self.reusable_pane(mode, depth, &snapshot) {
            apply_snapshot(pane, &snapshot, &self.browser);
            return;
        }
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.rebuild_icons(),
            BrowserMode::List => self.rebuild_list(),
            BrowserMode::Compact => self.rebuild_compact(),
        }
    }

    fn clear_mode(&mut self, mode: BrowserMode) {
        match mode {
            BrowserMode::Columns => {}
            BrowserMode::Icons => self.clear_icons(),
            BrowserMode::List => self.clear_list(),
            BrowserMode::Compact => self.clear_compact(),
        }
    }

    fn reusable_pane(
        &self,
        mode: BrowserMode,
        depth: usize,
        snapshot: &BrowserColumnSnapshot,
    ) -> Option<&Pane> {
        match mode {
            BrowserMode::Columns => None,
            BrowserMode::Icons => self
                .icons_panes
                .first()
                .filter(|pane| pane.shows(depth, &snapshot.location)),
            BrowserMode::List => self
                .list_pane
                .as_ref()
                .filter(|pane| self.list_pane_is_current(pane, depth, snapshot)),
            BrowserMode::Compact => self
                .compact_pane
                .as_ref()
                .filter(|pane| pane.shows(depth, &snapshot.location)),
        }
    }

    /// The browser decides which listings show the groups, so its automatic
    /// cursor lands on the first row this pane shows.
    fn sync_type_grouping(&self) {
        self.browser
            .set_type_grouping(self.group_by_type && self.mode.supports_type_grouping());
    }

    /// The List pane already shows this folder with its grouping and sort.
    fn list_pane_is_current(
        &self,
        pane: &Pane,
        depth: usize,
        snapshot: &BrowserColumnSnapshot,
    ) -> bool {
        if !pane.shows(depth, &snapshot.location) {
            return false;
        }
        if pane.group_by_type != self.browser.shows_type_groups(depth) {
            return false;
        }
        pane.sorting.as_ref().map(|sorting| sorting.get())
            == self
                .browser
                .column_preferences(depth)
                .map(|preferences| (preferences.sort_key, preferences.sort_direction))
    }

    pub fn show_mode(&self, mode: BrowserMode) {
        self.stack.set_visible_child_name(mode.page_name());
    }

    pub fn clear_inactive_mode(&mut self, mode: BrowserMode) {
        if self.mode == mode {
            return;
        }
        self.mode_panes(mode)
            .into_iter()
            .for_each(deactivate_pane_models);
    }

    pub(super) fn clear_icons(&mut self) {
        self.pane_navigation.borrow_mut().cancel();
        for pane in &self.icons_panes {
            detach_pane_models(pane);
        }
        clear_box(&self.icons_root);
        self.icons_panes.clear();
    }

    pub(super) fn clear_list(&mut self) {
        self.pane_navigation.borrow_mut().cancel();
        if let Some(pane) = self.list_pane.as_ref() {
            detach_pane_models(pane);
        }
        clear_box(&self.list_root);
        self.list_pane = None;
    }

    pub(super) fn clear_compact(&mut self) {
        self.pane_navigation.borrow_mut().cancel();
        if let Some(pane) = self.compact_pane.as_ref() {
            detach_pane_models(pane);
        }
        clear_box(&self.compact_root);
        self.compact_pane = None;
    }

    fn pane_build(
        &self,
        depth: usize,
        title: String,
        activation: &Rc<Cell<ClickActivation>>,
    ) -> PaneBuild {
        PaneBuild {
            browser: self.browser.clone(),
            depth,
            title,
            click: ModeClickOptions {
                previews: self.single_click_previews.clone(),
                activation: activation.clone(),
                multiple_selection: self.multiple_selection.clone(),
            },
            transfer: self.transfer_handler.clone(),
            marks: self.clipboard_marks.clone(),
            state: self.context_state.borrow().clone(),
            new_folder_state: self.new_folder_state.borrow().clone(),
            search_selection_handlers: self.search_selection_handlers.clone(),
            search_context_state: self.context_state.clone(),
        }
    }

    pub(super) fn rebuild_icons(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_icons();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_icons();
        let size = PreferenceManager::shared().icons_size_for(Some(&snapshot.location));
        self.icons_resolved_size.set(size);
        self.icons_thumbnail_size.set(size);
        let inputs = self.pane_build(
            depth,
            snapshot.location.display_name(),
            &self.icons_click_activation,
        );
        let mut pane = build_icons_pane(
            &inputs,
            IconsOptions {
                thumbnail_size: self.icons_thumbnail_size.clone(),
                group_by_type: false,
                density: self.density,
                layout: IconsLayout::Grid,
            },
        );
        install_pane_surface(&pane, &self.browser, &snapshot.location.display_name());
        configure_icons_density(&pane, self.density);
        pane.folder_context_trigger = self.install_context_menu(&pane);
        self.icons_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.pane_navigation.borrow_mut().prepare(&pane, &snapshot);
        self.icons_panes.push(pane);
    }

    pub(super) fn rebuild_compact(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_compact();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_compact();
        let inputs = self.pane_build(
            depth,
            snapshot.location.display_name(),
            &self.icons_click_activation,
        );
        let mut pane = build_icons_pane(
            &inputs,
            IconsOptions {
                thumbnail_size: Rc::new(Cell::new(compact_icon_size(self.density))),
                group_by_type: false,
                density: self.density,
                layout: IconsLayout::Compact,
            },
        );
        install_pane_surface(&pane, &self.browser, &snapshot.location.display_name());
        pane.folder_context_trigger = self.install_context_menu(&pane);
        self.compact_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.pane_navigation.borrow_mut().prepare(&pane, &snapshot);
        self.compact_pane = Some(pane);
    }

    pub(super) fn rebuild_list(&mut self) {
        let Some(depth) = self.browser.active_depth() else {
            self.clear_list();
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        self.clear_list();
        let inputs = self.pane_build(
            depth,
            snapshot.location.display_name(),
            &self.list_click_activation,
        );
        let mut pane = build_list_pane(&inputs, self.browser.shows_type_groups(depth));
        install_pane_surface(&pane, &self.browser, &snapshot.location.display_name());
        pane.folder_context_trigger = self.install_context_menu(&pane);
        self.list_root.append(&pane.shell);
        apply_snapshot(&pane, &snapshot, &self.browser);
        self.pane_navigation.borrow_mut().prepare(&pane, &snapshot);
        self.list_pane = Some(pane);
    }

    pub fn set_group_by_type(&mut self, enabled: bool) {
        if self.group_by_type == enabled {
            return;
        }
        self.cancel_rename();
        self.group_by_type = enabled;
        self.sync_type_grouping();
        if self.mode.supports_type_grouping() {
            self.rebuild_list();
        }
    }
}
