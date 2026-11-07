// SPDX-License-Identifier: MIT

use std::{rc::Rc, time::Duration};

use crate::model::{Location, SortDirection, SortKey, ViewPreferences};

use super::metadata_fills::SortPass;
use super::{Browser, BrowserEvent, PublicationPlan, PublishTerminal, SortOrigin};

impl Browser {
    pub fn requested_column_preferences(&self, depth: usize) -> Option<ViewPreferences> {
        let mut preferences = self.column_preferences(depth)?;
        // Only a sort still waiting to apply differs from the column; an applied one stays
        // pending after its column shows another folder.
        let location = self.location_at(depth);
        if let Some((requested, requested_location)) = self.requested_sort.borrow().as_ref()
            && requested.depth == depth
            && self.pending_sort.get() == Some((requested.generation, depth))
            && !self.pending_sort_applied.get()
            && location.as_ref() == Some(requested_location)
        {
            preferences.set_sort_options(requested.preferences.sort_options());
        }
        if !self
            .location_at(depth)
            .is_some_and(|location| location.is_recent_root())
        {
            preferences.folders_first = self.preferences.get().folders_first;
        }
        Some(preferences)
    }

    pub fn restore_default_sort(self: &Rc<Self>, depth: usize, options: crate::model::SortOptions) {
        self.apply_column_preferences(depth, SortOrigin::Followed, move |preferences| {
            preferences.set_sort_options(options);
        });
    }

    pub fn set_sort_key(self: &Rc<Self>, depth: usize, sort_key: SortKey) {
        if sort_key == SortKey::Recency
            && !self
                .location_at(depth)
                .is_some_and(|location| location.is_recent_root())
        {
            return;
        }
        if sort_key == SortKey::DeviceOrder {
            let mut state = self.state.borrow_mut();
            if !state
                .columns
                .get(depth)
                .is_some_and(|column| column.location.is_camera_photo_root())
            {
                return;
            }
            let Some(mut preferences) = state.column_preferences(depth) else {
                return;
            };
            preferences.sort_key = SortKey::DeviceOrder;
            state.apply_sort_preferences(depth, preferences);
            drop(state);
            // A previously sorted list no longer carries its enumeration order.
            self.refresh_column(depth);
            return;
        }
        self.apply_column_preferences(depth, SortOrigin::Chosen, move |preferences| {
            preferences.sort_key = sort_key;
        });
    }

    pub fn set_sort(
        self: &Rc<Self>,
        depth: usize,
        sort_key: SortKey,
        sort_direction: SortDirection,
    ) {
        if sort_key == SortKey::Recency
            && !self
                .location_at(depth)
                .is_some_and(|location| location.is_recent_root())
        {
            return;
        }
        if matches!(sort_key, SortKey::DeviceOrder | SortKey::Unsorted) {
            self.set_sort_key(depth, sort_key);
            return;
        }
        self.apply_column_preferences(depth, SortOrigin::Chosen, move |preferences| {
            preferences.sort_key = sort_key;
            preferences.sort_direction = sort_direction;
        });
    }

    fn reload_discovery_order(self: &Rc<Self>, pass: SortPass) {
        self.sort_resync
            .note_attempt(pass.origin, pass.depth, pass.preferences);
        self.state
            .borrow_mut()
            .apply_sort_preferences(pass.depth, pass.preferences);
        if pass.origin == SortOrigin::Chosen {
            self.record_sort(pass.depth);
        }
        self.pending_sort.set(None);
        self.pending_sort_applied.set(false);
        self.finish_sorting(pass.depth);
        self.refresh_column(pass.depth);
    }

    pub fn set_name_comparison(
        self: &Rc<Self>,
        depth: usize,
        comparison: crate::model::NameComparison,
    ) {
        if self
            .requested_column_preferences(depth)
            .is_some_and(|prefs| prefs.sort_key == SortKey::Unsorted)
        {
            return;
        }
        self.apply_column_preferences(depth, SortOrigin::Chosen, move |preferences| {
            preferences.name_comparison = comparison;
        });
    }

    pub fn set_sort_direction(self: &Rc<Self>, depth: usize, sort_direction: SortDirection) {
        if self
            .requested_column_preferences(depth)
            .is_some_and(|prefs| prefs.sort_key == SortKey::Unsorted)
        {
            return;
        }
        self.apply_column_preferences(depth, SortOrigin::Chosen, move |preferences| {
            preferences.sort_direction = sort_direction;
        });
    }

    /// Folders-first is application-wide: every column re-sorts, `depth` first.
    pub fn set_folders_first(self: &Rc<Self>, depth: usize, folders_first: bool) {
        if self.preferences.get().folders_first == folders_first {
            return;
        }
        self.update_defaults(|defaults| defaults.folders_first = folders_first);
        self.resync_column_sorts_from(Some(depth));
    }

    /// Recent never lists folders first, whatever its sort. Returns whether the setting changed.
    pub fn toggle_folders_first(self: &Rc<Self>, depth: usize) -> bool {
        if self
            .location_at(depth)
            .is_none_or(|location| location.is_recent_root())
        {
            return false;
        }
        self.set_folders_first(depth, !self.preferences.get().folders_first);
        true
    }

    pub(super) fn update_defaults(&self, change: impl FnOnce(&mut ViewPreferences)) {
        let mut defaults = self.preferences.get();
        change(&mut defaults);
        self.preferences.set(defaults);
        self.state.borrow_mut().set_default_preferences(defaults);
        self.notify_preferences_observers();
    }

    pub fn apply_default_preferences(self: &Rc<Self>, preferences: ViewPreferences) {
        let previous = self.preferences.replace(preferences);
        self.state.borrow_mut().set_default_preferences(preferences);
        if previous.show_hidden != preferences.show_hidden {
            self.close_peek();
            self.state
                .borrow_mut()
                .set_show_hidden(preferences.show_hidden);
            self.emit(BrowserEvent::HiddenToggled {
                show_hidden: preferences.show_hidden,
            });
        }
        if previous != preferences {
            self.notify_preferences_observers();
        }
        if (previous.sort_options(), previous.folders_first)
            != (preferences.sort_options(), preferences.folders_first)
        {
            self.resync_column_sorts();
        }
    }

    pub fn toggle_hidden(self: &Rc<Self>) {
        let mut preferences = self.preferences.get();
        preferences.show_hidden = !preferences.show_hidden;
        self.preferences.set(preferences);
        self.notify_preferences_observers();

        self.close_peek();
        self.state
            .borrow_mut()
            .set_show_hidden(preferences.show_hidden);
        self.emit(BrowserEvent::HiddenToggled {
            show_hidden: preferences.show_hidden,
        });
    }

    pub(super) fn apply_column_preferences(
        self: &Rc<Self>,
        depth: usize,
        origin: SortOrigin,
        update: impl FnOnce(&mut ViewPreferences) + 'static,
    ) {
        let Some(location) = self.location_at(depth) else {
            return;
        };
        let Some(mut requested) = self.requested_column_preferences(depth) else {
            return;
        };
        update(&mut requested);
        let generation = self.begin_sort(depth);
        self.requested_sort.replace(Some((
            SortPass {
                depth,
                generation,
                origin,
                preferences: requested,
            },
            location.clone(),
        )));
        let weak = Rc::downgrade(self);
        gio::glib::timeout_add_local_once(Duration::from_millis(16), move || {
            if let Some(browser) = weak.upgrade() {
                browser.apply_debounced_sort(
                    depth,
                    &location,
                    generation,
                    origin,
                    move |preferences| {
                        preferences.set_sort_options(requested.sort_options());
                        preferences.folders_first = requested.folders_first;
                    },
                );
            }
        });
    }

    /// A fresh generation per sort, so a stale timer or fill cannot pass for a newer sort.
    /// An applied sort has already finished, or finishes when its rows publish.
    pub(super) fn begin_sort(&self, depth: usize) -> u64 {
        let generation = self.sort_generation.get().wrapping_add(1);
        self.sort_generation.set(generation);
        let previous_applied = self.pending_sort_applied.replace(false);
        if let Some((_, previous_depth)) = self.pending_sort.replace(Some((generation, depth)))
            && !previous_applied
        {
            self.finish_sorting(previous_depth);
        }
        self.emit(BrowserEvent::SortingStarted { depth });
        generation
    }

    /// Pairs each `SortingStarted`, and lets a queued re-sync sort its next column.
    pub(in crate::app) fn finish_sorting(&self, depth: usize) {
        self.emit(BrowserEvent::SortingFinished { depth });
        self.wake_sort_resync();
    }

    pub(super) fn apply_debounced_sort(
        self: &Rc<Self>,
        depth: usize,
        location: &Location,
        generation: u64,
        origin: SortOrigin,
        update: impl FnOnce(&mut ViewPreferences),
    ) {
        if self.pending_sort.get() != Some((generation, depth)) {
            return;
        }
        let result = {
            let mut state = self.state.borrow_mut();
            // The column may now show another folder, which must not take this sort.
            let Some(mut preferences) = state
                .column_preferences(depth)
                .filter(|_| state.location_at(depth).as_ref() == Some(location))
            else {
                drop(state);
                self.pending_sort.set(None);
                self.finish_sorting(depth);
                return;
            };
            let previous = preferences;
            update(&mut preferences);
            if preferences.sort_key == SortKey::Unsorted
                && (previous.sort_key != SortKey::Unsorted
                    || previous.folders_first != preferences.folders_first)
            {
                drop(state);
                self.reload_discovery_order(SortPass {
                    depth,
                    generation,
                    origin,
                    preferences,
                });
                return;
            }
            // Size/date sorts need the metadata streaming enumeration skipped:
            // fill the whole column first instead of sorting placeholders.
            let targets = state
                .column_unknown_sort_metadata(depth, preferences.sort_key)
                .unwrap_or_default();
            if matches!(
                preferences.sort_key,
                SortKey::Size | SortKey::Modified | SortKey::Created
            ) && !targets.is_empty()
            {
                drop(state);
                let pass = SortPass {
                    depth,
                    generation,
                    origin,
                    preferences,
                };
                self.request_sort_fill(pass, targets);
                return;
            }
            self.sort_resync.note_attempt(origin, depth, preferences);
            let result = state.apply_sort_preferences(depth, preferences);
            self.pending_sort_applied.set(true);
            let request_id = state.request_id_for_depth(depth);
            let total = state.columns.get(depth).map(|column| column.entries.len());
            let take_focus = state.take_selection_from_reveal(depth);
            result.and_then(|(focused, positions)| {
                Some(PublicationPlan {
                    request_id: request_id?,
                    total: total?,
                    focused,
                    positions,
                    take_focus,
                    terminal: PublishTerminal::SortingFinished,
                })
            })
        };
        if origin == SortOrigin::Chosen {
            self.record_sort(depth);
        }
        if let Some(plan) = result {
            self.publish_staged(depth, plan);
        } else {
            self.pending_sort.set(None);
            self.finish_sorting(depth);
        }
    }
}
