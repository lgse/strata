// SPDX-License-Identifier: MIT

//! View modes, presentation settings and the focus a view switch restores.

use super::*;

impl BrowserView {
    pub fn view_mode(&self) -> BrowserMode {
        self.state.mode.get()
    }

    pub fn connect_view_mode_changed(&self, handler: impl Fn(BrowserMode) + 'static) {
        let state = Rc::downgrade(&self.state);
        self.state
            .mode_views
            .borrow()
            .widget()
            .connect_visible_child_name_notify(move |stack| {
                let mode = BrowserMode::from_page_name(
                    stack.visible_child_name().as_deref().unwrap_or_default(),
                );
                if let Some(state) = state.upgrade() {
                    state.enter_mode(mode);
                }
                handler(mode);
            });
    }

    pub fn set_view_mode(&self, mode: BrowserMode) {
        let previous = self.state.mode.get();
        if mode == previous {
            return;
        }
        if mode == BrowserMode::Columns {
            self.state.cancel_peek();
            self.state.peek_anchor.take();
        }
        // The rebuilt view has a different displayed order for the same anchor.
        self.state.browser.leave_visual();
        let searching = self.state.listing_search_showing();
        // Read before the rebuild tears down the focused field or results.
        let filter_focus = self.state.filter_focus();
        self.state.enter_mode(mode);
        let filter = match previous {
            BrowserMode::Columns => self.state.capture_active_column_filter(),
            BrowserMode::Icons | BrowserMode::List | BrowserMode::Compact => {
                self.state.mode_views.borrow().capture_active_filter()
            }
        };
        self.state.mode_views.borrow().show_mode(mode);
        self.state.mode_views.borrow_mut().prepare_mode(mode);
        if mode == BrowserMode::Columns {
            self.rebuild_columns_view(&filter, searching);
        } else {
            self.state
                .mode_views
                .borrow()
                .restore_active_filter(&filter);
        }
        match previous {
            // The columns view is already hidden, so an exit animation here
            // would never be seen.
            BrowserMode::Columns => self.state.truncate_for_replacement(0),
            BrowserMode::Icons | BrowserMode::List | BrowserMode::Compact => self
                .state
                .mode_views
                .borrow_mut()
                .clear_inactive_mode(previous),
        }
        self.state.sync_displayed_location();
        self.state.carry_listing_search(searching);
        self.restore_view_switch_focus(filter_focus);
    }

    /// Rebuilds Columns with the active column's child following the selection
    /// the other view left: no child for an empty selection, and a newly
    /// selected folder opens as the child while Columns mirrors selection.
    fn rebuild_columns_view(&self, filter: &ActivePaneFilter, searching: bool) {
        let selected_child = self.state.browser.follow_selection_on_return();
        self.state.rebuild_columns_from(0);
        self.state.restore_active_column_filter(filter);
        // Opening a column ends a listing search, which the switch carries over.
        let opens = !searching && self.state.columns_mirror_selection.get();
        if opens && let Some((depth, location)) = selected_child {
            self.state.browser.show_child(depth, location);
        }
    }

    /// Focus follows the filter session across a view switch.
    pub(super) fn restore_view_switch_focus(&self, filter_focus: Option<FilterFocus>) {
        if !crate::ui::focus_navigation::browser_can_take_focus(&self.state.overlay) {
            return;
        }
        if let Some(target) = self.filter_target() {
            if filter_focus == Some(FilterFocus::Entry) {
                refocus_filter_entry(target.entry());
                return;
            }
            if filter_focus == Some(FilterFocus::Results)
                || listing_filter::filter_shows_query(&target)
            {
                target.settle();
                self.focus_filter_results(&target);
                return;
            }
        }
        if self.view_mode() == BrowserMode::Columns {
            self.state.focus_rebuilt_active_column();
        } else if let Some(depth) = self.state.browser.active_depth() {
            self.state.mode_views.borrow().focus_visible_pane(depth);
        }
    }

    /// Focus for a view switched from a menu, once the menu has closed: results replace
    /// the listing that would otherwise take focus.
    pub(in crate::ui) fn focus_switched_view(&self) {
        if let Some(target) = self
            .filter_target()
            .filter(listing_filter::filter_shows_query)
        {
            self.focus_filter_results(&target);
        } else {
            self.state.browser.focus_active();
        }
    }

    // Columns grabs the collection view itself; item-level focus lands on
    // editable cells that would swallow navigation keys.
    pub(in crate::ui) fn focus_file_view(&self) {
        if self.view_mode() == BrowserMode::Columns {
            self.state.focus_rebuilt_active_column();
        } else {
            self.state.browser.focus_active();
        }
    }

    pub(in crate::ui) fn restore_listing_focus(&self) {
        self.state.restore_listing_focus();
    }

    pub(in crate::ui) fn cancel_column_drag(&self) -> bool {
        self.state.mode_views.borrow().cancel_column_drag()
    }

    /// Makes this listing the focus target when a dismissed overlay leaves `window`
    /// without a focused widget.
    pub(in crate::ui) fn set_as_modal_focus_fallback(&self, window: &gtk::Window) {
        let view = self.downgrade();
        crate::ui::modal::set_modal_focus_fallback(
            window,
            Rc::new(move || {
                if let Some(view) = view.upgrade() {
                    view.restore_listing_focus();
                }
            }),
        );
    }

    pub fn set_density(&self, density: BrowserDensity) {
        self.state.mode_views.borrow_mut().set_density(density);
        self.state.overlay.remove_css_class("density-compact");
        self.state.overlay.remove_css_class("density-airy");
        self.state.overlay.remove_css_class("density-custom");
        self.state.overlay.add_css_class(match density {
            BrowserDensity::Compact => "density-compact",
            BrowserDensity::Airy => "density-airy",
            BrowserDensity::Custom(_) => "density-custom",
        });
    }

    /// Groups List entries under file-type headings. Icons and Columns keep the
    /// preference but do not apply it.
    pub fn set_group_by_type(&self, enabled: bool) {
        self.state
            .mode_views
            .borrow_mut()
            .set_group_by_type(enabled);
    }

    pub fn set_list_columns(&self) {
        self.state.mode_views.borrow().set_list_columns();
    }

    pub fn open_list_columns_popover(&self) {
        self.state.mode_views.borrow().open_list_columns_popover();
    }

    pub fn apply_resolved_icons_size(&self, size: i32) {
        self.state
            .mode_views
            .borrow_mut()
            .apply_resolved_icons_size(size);
    }

    /// Resizes Icons thumbnails; false outside Icons.
    pub(in crate::ui) fn resize_icons_thumbnails(
        &self,
        change: crate::ui::browser_modes::ThumbnailResize,
    ) -> bool {
        // Released first: the slider's change reaches the mode views through preferences.
        let target = self.state.mode_views.borrow().icons_thumbnail_scale();
        let Some((scale, location)) = target else {
            return false;
        };
        let size = change.apply(&scale, location.as_ref());
        self.show_size_notice(&rust_i18n::t!("Thumbnail size: %{size}px", size = size));
        true
    }

    pub(in crate::ui) fn set_text_size(&self, size: crate::ui::preferences::TextSize) {
        crate::ui::preferences::PreferenceManager::shared().set_text_size(size);
        self.show_size_notice(&rust_i18n::t!(
            "Text size: %{size}px",
            size = size.root_font_px()
        ));
    }

    pub(super) fn show_size_notice(&self, text: &str) {
        crate::ui::notice::show_status(&self.state.overlay, text, SIZE_NOTICE_DURATION);
    }

    pub fn set_peek_enabled(&self, enabled: bool) {
        self.state.peek_enabled.set(enabled);
        if !enabled {
            cancel_source(&self.state.pending_peek);
            self.state.browser.close_peek();
        }
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.state.single_click_previews.set(enabled);
        self.state
            .mode_views
            .borrow()
            .set_single_click_previews(enabled);
    }

    pub fn set_columns_mirror_selection(&self, enabled: bool) {
        self.state.columns_mirror_selection.set(enabled);
    }

    #[cfg(test)]
    pub(in crate::ui) fn columns_mirror_selection_enabled(&self) -> bool {
        self.state.columns_mirror_selection.get()
    }

    /// Runs the cursor mirror at once instead of after its delay.
    #[cfg(test)]
    pub(in crate::ui) fn mirror_focused_folder_now(&self) {
        if let Some((depth, position, _)) = self.state.browser.focused_item() {
            self.state.apply_child_mirror(depth, position, false);
        }
    }

    pub fn set_click_activation(&self, mode: BrowserMode, activation: ClickActivation) {
        if mode == BrowserMode::Columns {
            self.state.columns_click_activation.set(activation);
        } else {
            self.state
                .mode_views
                .borrow()
                .set_click_activation(mode, activation);
        }
    }

    pub(in crate::ui) fn set_chooser_allows_create(&self, allow: bool) {
        self.state.chooser_allows_create.set(allow);
    }

    pub(in crate::ui) fn set_preview_owns_keys(&self, owned: bool) {
        if self.state.preview_owns_keys.replace(owned) != owned {
            self.state.refresh_destination_style();
        }
    }

    pub(in crate::ui) fn set_playback_handoff(&self, handoff: PlaybackHandoff) {
        self.state.playback_handoff.replace(Some(handoff));
    }

    pub(in crate::ui) fn record_pointer_hover(&self, surface: (f64, f64), column: Option<usize>) {
        if self
            .state
            .input_ownership
            .borrow_mut()
            .pointer_motion(surface)
        {
            self.state.adopt_pointer(column);
        }
    }

    pub fn toggle_folder_peek(&self) {
        if self.state.peek.borrow().is_some() {
            self.state.browser.close_peek();
            self.keyboard_navigation();
            return;
        }
        self.keyboard_navigation();
        self.state.open_keyboard_peek();
    }

    pub fn set_watch_network_folders(&self, enabled: bool) {
        let browser = &self.state.browser;
        browser.set_freshness(FreshnessInput {
            watch_network_folders: enabled,
            ..browser.freshness_input()
        });
    }

    pub fn set_presence(&self, presence: Presence, power_saver: bool) {
        self.state.path_completion.set_window_presence(presence);
        let browser = &self.state.browser;
        browser.set_freshness(FreshnessInput {
            presence,
            power_saver,
            ..browser.freshness_input()
        });
    }
}
