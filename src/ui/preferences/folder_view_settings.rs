// SPDX-License-Identifier: MIT

use std::{cell::RefCell, path::Path, time::Duration};

use gtk::glib;

use crate::{
    adapters::{Watched, probes},
    model::{FolderSort, Location, SortDirection, SortKey},
    services::{ItemIdentity, LoadHandle},
};

use strata_desktop::settings::{
    FOLDER_VIEWS_SAVE_DELAY as SAVE_DELAY, FOLDER_VIEWS_USE_SAVE_DELAY as USE_SAVE_DELAY,
    FolderViewStore as SharedStore, UnreadableAtStartup, Written,
    folder_view_policy::{self as policy, IconsSizeKept},
};
// The tests name the path here.
pub(in crate::ui) use strata_desktop::settings::folder_views_path;

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, PreferenceManager, SHARED_MANAGER,
    folder_views::{FolderKey, FolderViews, TakenViews, key_for_location},
    parse_preferences, settings_path, store_sort,
    trashed::TrashedValues,
};
use crate::storage::writer;

/// Whether a change can reach saved sorts, which re-sort open columns.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Affects {
    Sorts,
    IconsSizesOnly,
}

/// The shared store, with what GTK adds to it: the timer a write waits on, the values of
/// folders in the trash, and the two monitors.
pub(super) struct FolderViewStore {
    shared: RefCell<SharedStore>,
    save_timer: RefCell<Option<(glib::SourceId, Duration)>>,
    trashed: RefCell<TrashedValues<TakenViews>>,
    /// Follow what another Strata process, such as the portal chooser, saves.
    _monitors: [LoadHandle; 2],
}

fn watch(path: &Path, changed: fn(&PreferenceManager)) -> LoadHandle {
    let notice = move || {
        if let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) {
            changed(&manager);
        }
    };
    crate::adapters::watch(
        &Location::local(path),
        Watched::File,
        move |_, _| notice(),
        notice,
    )
}

impl FolderViewStore {
    fn load() -> Self {
        let shared = SharedStore::load();
        let path = folder_views_path();
        Self {
            shared: RefCell::new(shared),
            save_timer: RefCell::new(None),
            trashed: RefCell::default(),
            _monitors: [
                watch(&path, PreferenceManager::merge_folder_views_saved_elsewhere),
                watch(
                    &settings_path(),
                    PreferenceManager::adopt_defaults_saved_elsewhere,
                ),
            ],
        }
    }
}

/// Writes pending folder settings, such as use times, before the process exits.
pub(crate) fn flush_pending_folder_views() {
    if let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) {
        manager.flush_folder_views();
    }
}

impl PreferenceManager {
    pub(super) fn folder_view_store(&self) -> &FolderViewStore {
        self.folder_views.get_or_init(FolderViewStore::load)
    }

    pub(in crate::ui) fn remember_folder_views(&self) -> bool {
        self.preferences.borrow().remember_folder_views
    }

    /// Defaults changed while this was off can equal saved values, which are
    /// dropped when it is turned back on.
    pub(in crate::ui) fn set_remember_folder_views(&self, enabled: bool) {
        let prune = policy::set_remember_folder_views(&mut self.preferences.borrow_mut(), enabled);
        if let Some(prune) = prune {
            self.change_folder_views(Affects::Sorts, |views| prune.apply(views));
        }
        self.save_preferences();
    }

    /// Changes whenever a saved folder sort may have changed, for bindings that resolve one.
    pub(in crate::ui) fn folder_sorts_revision(&self) -> u64 {
        self.folder_sorts_revision.get()
    }

    fn remembered_key(&self, location: &Location) -> Option<FolderKey> {
        policy::remembered_key(&self.preferences.borrow(), location)
    }

    pub(in crate::ui) fn remembers(&self, location: &Location) -> bool {
        self.remembered_key(location).is_some()
    }

    /// `opened` counts as using the folder for the least-recently-used limit.
    pub(in crate::ui) fn resolve_folder_sort(
        &self,
        location: &Location,
        opened: bool,
    ) -> FolderSort {
        let Some(key) = self.remembered_key(location) else {
            return FolderSort::Unremembered;
        };
        let store = self.folder_view_store();
        let (touched, sort) = {
            let mut shared = store.shared.borrow_mut();
            let views = shared.views_mut();
            (opened && views.touch(&key), views.view(&key).sort)
        };
        if touched {
            self.schedule_folder_views_write(store, USE_SAVE_DELAY);
        }
        match sort {
            Some((key, direction, comparison)) => FolderSort::Saved(key, direction, comparison),
            None => FolderSort::Default,
        }
    }

    /// A sort equal to the default is not stored, so the folder follows later defaults.
    #[cfg(test)]
    pub(in crate::ui) fn set_folder_sort(
        &self,
        location: &Location,
        sort_key: SortKey,
        sort_direction: SortDirection,
    ) {
        let comparison = crate::model::NameComparison::default();
        self.set_folder_sort_options(location, (sort_key, sort_direction, comparison));
    }

    pub(in crate::ui) fn set_folder_sort_options(
        &self,
        location: &Location,
        options: crate::model::SortOptions,
    ) {
        if options.0.stored_name().is_none() {
            return;
        }
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        let sort = Some(options).filter(|sort| *sort != self.default_sort_options());
        self.change_folder_views(Affects::Sorts, |views| {
            views.update(&key, |view| view.sort = sort)
        });
    }

    pub(in crate::ui) fn reset_folder_sort(&self, location: &Location) {
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        self.change_folder_views(Affects::Sorts, |views| {
            views.update(&key, |view| view.sort = None)
        });
    }

    pub(in crate::ui) fn default_sort(&self) -> (SortKey, SortDirection) {
        self.sort_preferences().sort()
    }

    pub(in crate::ui) fn set_default_sort(&self, sort_key: SortKey, sort_direction: SortDirection) {
        let comparison = self.sort_preferences().name_comparison;
        self.set_default_sort_options((sort_key, sort_direction, comparison));
    }

    pub(in crate::ui) fn default_sort_options(&self) -> crate::model::SortOptions {
        self.sort_preferences().sort_options()
    }

    pub(in crate::ui) fn set_default_sort_options(&self, options: crate::model::SortOptions) {
        if !store_sort(&mut self.preferences.borrow_mut(), options) {
            return;
        }
        self.save_preferences();
        if self.remember_folder_views() {
            self.change_folder_views(Affects::Sorts, |views| views.prune_default_sort(options));
        }
    }

    pub(in crate::ui) fn icons_size_for(&self, location: Option<&Location>) -> i32 {
        policy::icons_size_for(&self.preferences.borrow(), location, |key| {
            self.folder_view_store()
                .shared
                .borrow()
                .views()
                .view(key)
                .icons_size
        })
    }

    /// A folder that is not remembered keeps a changed size only while it is shown.
    pub(in crate::ui) fn set_folder_icons_size(&self, location: Option<&Location>, size: i32) {
        let kept = policy::keep_icons_size(&self.preferences.borrow(), location, size);
        match kept {
            IconsSizeKept::Default(size) => self.set_icons_thumbnail_size(size),
            IconsSizeKept::Folder(key, size) => {
                self.change_folder_views(Affects::IconsSizesOnly, |views| {
                    views.update(&key, |view| view.icons_size = size)
                });
            }
            IconsSizeKept::Shown => {}
        }
    }

    pub(in crate::ui) fn reset_folder_icons_size(&self, location: &Location) {
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        self.change_folder_views(Affects::IconsSizesOnly, |views| {
            views.update(&key, |view| view.icons_size = None)
        });
    }

    pub(in crate::ui) fn set_default_icons_size(&self, size: i32) {
        let prune = policy::set_default_icons_size(&mut self.preferences.borrow_mut(), size);
        self.save_preferences();
        if let Some(prune) = prune {
            self.change_folder_views(Affects::IconsSizesOnly, |views| prune.apply(views));
        }
    }

    pub(in crate::ui) fn has_folder_views(&self) -> bool {
        !self.folder_view_store().shared.borrow().views().is_empty()
    }

    /// Also clears what another Strata process saved and this one has not merged yet.
    pub(in crate::ui) fn forget_folder_views(&self) {
        self.change_folder_views(Affects::Sorts, |views| {
            views.clear();
            true
        });
    }

    /// Saved values follow a folder renamed or moved within Strata, whether or not
    /// per-folder settings are in use, and are dropped when it moves somewhere
    /// that is not remembered.
    pub(in crate::ui) fn relocate_folder_views(
        &self,
        from: &Location,
        to: &Location,
        merged: bool,
    ) {
        let Some(from) = key_for_location(from) else {
            return;
        };
        let to = key_for_location(to);
        self.change_folder_views(Affects::Sorts, |views| {
            views.relocate(&from, to.as_ref(), merged)
        });
    }

    /// Drops the values of a removed folder and of the folders inside it. A
    /// `trash_identity` keeps them for [`Self::restore_folder_views`] until the
    /// session ends.
    pub(in crate::ui) fn forget_folder_views_within(
        &self,
        location: &Location,
        trash_identity: Option<ItemIdentity>,
    ) {
        let Some(key) = key_for_location(location) else {
            return;
        };
        let store = self.folder_view_store();
        if let Some(identity) = trash_identity {
            store.trashed.borrow_mut().forget_identity(identity);
        }
        let taken = store.shared.borrow_mut().views_mut().take_within(&key);
        let keys = taken.len();
        if keys == 0 {
            return;
        }
        if let (Some(identity), Some(root)) = (trash_identity, location.native_path()) {
            store
                .trashed
                .borrow_mut()
                .keep(root.to_path_buf(), identity, taken, keys);
        }
        self.folder_views_changed(store, Affects::Sorts);
    }

    pub(in crate::ui) fn restore_folder_views(
        &self,
        location: &Location,
        identity: Option<ItemIdentity>,
    ) {
        let (Some(store), Some(root), Some(identity)) =
            (self.folder_views.get(), location.native_path(), identity)
        else {
            return;
        };
        let taken = store.trashed.borrow_mut().take_restored(root, identity);
        if let Some(taken) = taken {
            self.change_folder_views(Affects::Sorts, |views| views.put_back(taken));
        }
    }

    /// A local folder that failed to open because it was deleted forgets its
    /// saved values and those of the folders inside it.
    pub(in crate::ui) fn forget_missing_folder(&self, location: &Location) {
        let (Some(path), Some(key)) = (location.native_path(), key_for_location(location)) else {
            return;
        };
        let store = self.folder_view_store();
        if !store.shared.borrow().views().may_forget(&key) {
            return;
        }
        let path = path.to_owned();
        glib::MainContext::default().spawn_local(async move {
            if !probes::folder_was_deleted(&path).await {
                return;
            }
            if let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) {
                manager
                    .change_folder_views(Affects::Sorts, |views| views.relocate(&key, None, false));
            }
        });
    }

    /// Another Strata process, such as the portal chooser, can change the
    /// defaults that folder values are stored relative to.
    fn adopt_defaults_saved_elsewhere(&self) {
        if self.saving.borrow().load_failure().is_some() {
            return;
        }
        let saved = writer::reread(settings_path(), parse_preferences);
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(Ok(Some(saved))), Some(manager)) = (saved.await, this.upgrade())
                && ![
                    "sort_key",
                    "sort_direction",
                    "natural_sort",
                    "case_sensitive_sort",
                    "icons_thumbnail_size",
                ]
                .iter()
                .any(|key| manager.setting_unsaved(key))
            {
                manager.adopt_defaults(saved);
            }
        });
    }

    fn adopt_defaults(&self, saved: super::Preferences) {
        let size = saved
            .icons_thumbnail_size
            .clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        {
            let mut preferences = self.preferences.borrow_mut();
            if preferences.sort_key == saved.sort_key
                && preferences.sort_direction == saved.sort_direction
                && preferences.natural_sort == saved.natural_sort
                && preferences.case_sensitive_sort == saved.case_sensitive_sort
                && preferences.icons_thumbnail_size == size
            {
                return;
            }
            preferences.sort_key = saved.sort_key;
            preferences.sort_direction = saved.sort_direction;
            preferences.natural_sort = saved.natural_sort;
            preferences.case_sensitive_sort = saved.case_sensitive_sort;
            preferences.icons_thumbnail_size = size;
            self.saving.borrow_mut().record_adopted(
                &preferences,
                &[
                    "sort_key",
                    "sort_direction",
                    "natural_sort",
                    "case_sensitive_sort",
                    "icons_thumbnail_size",
                ],
            );
        }
        if self.changes.record(&self.preferences.borrow()) {
            self.changes.notify(self);
        }
    }

    fn merge_folder_views_saved_elsewhere(&self) {
        if self.folder_views.get().is_none() {
            return;
        }
        let saved = SharedStore::reread();
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            if let (Some(saved), Some(manager)) = (saved.await, this.upgrade()) {
                manager.adopt_folder_views(saved);
            }
        });
    }

    #[cfg(test)]
    pub(in crate::ui) fn notice_saves_elsewhere(&self) {
        self.merge_folder_views_saved_elsewhere();
        self.adopt_defaults_saved_elsewhere();
    }

    fn adopt_folder_views(&self, saved: FolderViews) {
        let Some(store) = self.folder_views.get() else {
            return;
        };
        let changed = store.shared.borrow_mut().adopt(saved);
        if changed {
            self.publish_folder_views(Affects::Sorts);
        }
    }

    pub(in crate::ui) fn flush_folder_views(&self) {
        if let Some(store) = self.folder_views.get()
            && store.shared.borrow().views().has_unsaved()
        {
            self.write_folder_views(store, true);
        }
    }

    fn change_folder_views(&self, affects: Affects, change: impl FnOnce(&mut FolderViews) -> bool) {
        let store = self.folder_view_store();
        if change(store.shared.borrow_mut().views_mut()) {
            self.folder_views_changed(store, affects);
        }
    }

    fn folder_views_changed(&self, store: &FolderViewStore, affects: Affects) {
        self.schedule_folder_views_write(store, SAVE_DELAY);
        self.publish_folder_views(affects);
    }

    fn publish_folder_views(&self, affects: Affects) {
        if affects == Affects::Sorts {
            self.folder_sorts_revision
                .set(self.folder_sorts_revision.get().wrapping_add(1));
        }
        self.changes.notify_unrecorded(self);
    }

    /// A pending write keeps its time unless a shorter wait is asked for.
    fn schedule_folder_views_write(&self, store: &FolderViewStore, delay: Duration) {
        if store.shared.borrow().load_failure().is_some() {
            self.write_folder_views(store, false);
            return;
        }
        let mut timer = store.save_timer.borrow_mut();
        if timer
            .as_ref()
            .is_some_and(|(_, scheduled)| *scheduled <= delay)
        {
            return;
        }
        if let Some((source, _)) = timer.take() {
            source.remove();
        }
        let source = glib::timeout_add_local_once(delay, || {
            let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) else {
                return;
            };
            if let Some(store) = manager.folder_views.get() {
                // This source is finishing, so it must not be removed again.
                store.save_timer.borrow_mut().take();
                manager.write_folder_views(store, false);
            }
        });
        *timer = Some((source, delay));
    }

    /// A save that finds another process saving is retried later, or at `exit`
    /// waits briefly and then goes ahead unlocked.
    fn write_folder_views(&self, store: &FolderViewStore, exit: bool) {
        if let Some((timer, _)) = store.save_timer.borrow_mut().take() {
            timer.remove();
        }
        let begun = store.shared.borrow().begin_write(exit);
        let write = match begun {
            Ok(write) => write,
            Err(UnreadableAtStartup { path, error }) => {
                self.folder_save_notices
                    .unreadable_at_startup(&path, &error);
                return;
            }
        };
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let written = write.finished().await;
            if let Some(manager) = this.upgrade() {
                manager.folder_views_saved(written);
            }
        });
    }

    fn folder_views_saved(&self, written: Written) {
        let Some(store) = self.folder_views.get() else {
            return;
        };
        match written {
            Written::Busy => self.schedule_folder_views_write(store, SAVE_DELAY),
            Written::Failed { path, error } => {
                self.folder_save_notices.write_failed(&path, &error);
            }
            Written::Saved { merged, write } => {
                self.folder_save_notices.saved();
                if let Some(merged) = merged {
                    self.adopt_folder_views(merged);
                }
                store.shared.borrow_mut().finish_write(write);
            }
        }
    }
}
