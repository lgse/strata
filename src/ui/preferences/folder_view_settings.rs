// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::{FolderSort, Location, SortDirection, SortKey},
    services::TrashedOriginal,
};

use super::{
    FileStamp, MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, PreferenceManager,
    SHARED_MANAGER, file_stamp,
    folder_views::{
        FolderKey, FolderViews, TakenViews, folder_was_deleted, holds_mount_points,
        key_for_location,
    },
    read_preferences, read_state_file, settings_path, store_sort,
    trashed::TrashedValues,
};

const FILE_NAME: &str = "folder-views.toml";
/// Coalesces bursts of changes, such as a thumbnail-size drag, into one write
/// that stays off the click that made them.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Opening folders only refreshes use times, which can wait for other changes.
const USE_SAVE_DELAY: Duration = Duration::from_secs(60);

/// Whether a change can reach saved sorts, which re-sort open columns.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Affects {
    Sorts,
    IconsSizesOnly,
}

pub(super) struct FolderViewStore {
    views: RefCell<FolderViews>,
    /// Why saving is off for this session: the file existed but could not be read.
    load_failure: Option<io::Error>,
    save_timer: RefCell<Option<(glib::SourceId, Duration)>>,
    /// The file as this process last read or wrote it; anything else on disk was
    /// written by another Strata process and is merged before saving.
    synced: Cell<Option<FileStamp>>,
    trashed: RefCell<TrashedValues<TakenViews>>,
    /// Follow what another Strata process, such as the portal chooser, saves.
    _monitors: [Option<gio::FileMonitor>; 2],
}

fn watch(path: &Path, changed: fn(&PreferenceManager)) -> Option<gio::FileMonitor> {
    let monitor = gio::File::for_path(path)
        .monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        .inspect_err(|error| {
            tracing::debug!(%error, path = %path.display(), "unable to watch for other processes");
        })
        .ok()?;
    monitor.connect_changed(move |_, _, _, _| {
        if let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) {
            changed(&manager);
        }
    });
    Some(monitor)
}

impl FolderViewStore {
    fn load() -> Self {
        let path = folder_views_path();
        let synced = file_stamp(&path);
        let loaded = read_state_file(&path).and_then(|contents| match contents {
            Some(contents) => FolderViews::parse(&contents),
            None => Ok((FolderViews::default(), false)),
        });
        let (views, load_failure) = match loaded {
            Ok((views, repaired)) => {
                if repaired {
                    tracing::warn!(path = %path.display(),
                        "folder settings file has invalid entries; keeping the valid ones");
                }
                (views, None)
            }
            Err(error) => (FolderViews::default(), Some(error)),
        };
        if let Some(error) = &load_failure {
            tracing::warn!(%error, path = %path.display(),
                "unable to load folder settings; using none without saving; fix the file and restart Strata");
        }
        Self {
            views: RefCell::new(views),
            load_failure,
            save_timer: RefCell::new(None),
            synced: Cell::new(synced),
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

    /// Takes in what another process saved since this one last read or wrote
    /// the file, and returns whether that changed any folder's values. An
    /// unreadable file, or one from a newer version, must not be overwritten.
    fn merge_saved_elsewhere(&self, path: &Path) -> io::Result<bool> {
        let Some(stamp) = file_stamp(path).filter(|stamp| Some(*stamp) != self.synced.get()) else {
            return Ok(false);
        };
        let Some(contents) = read_state_file(path)? else {
            return Ok(false);
        };
        let (saved, _) = FolderViews::parse(&contents)?;
        let merged = self.views.borrow().merged_over(saved);
        let changed = !merged.same_views(&self.views.borrow());
        self.views.replace(merged);
        self.synced.set(Some(stamp));
        Ok(changed)
    }
}

pub(in crate::ui) fn folder_views_path() -> PathBuf {
    crate::storage::state_directory().join(FILE_NAME)
}

/// Writes pending folder settings, such as use times, before the process exits.
pub(crate) fn flush_pending_folder_views() {
    if let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) {
        manager.flush_folder_views();
    }
}

impl PreferenceManager {
    fn folder_view_store(&self) -> &FolderViewStore {
        self.folder_views.get_or_init(FolderViewStore::load)
    }

    pub(in crate::ui) fn remember_folder_views(&self) -> bool {
        self.preferences.borrow().remember_folder_views
    }

    /// Defaults changed while this was off can equal saved values, which are
    /// dropped when it is turned back on.
    pub(in crate::ui) fn set_remember_folder_views(&self, enabled: bool) {
        if enabled {
            let (sort, size) = (self.default_sort(), self.icons_thumbnail_size());
            self.change_folder_views(Affects::Sorts, |views| {
                views.prune_default_sort(sort) | views.prune_default_icons_size(size)
            });
        }
        self.preferences.borrow_mut().remember_folder_views = enabled;
        self.save_preferences();
    }

    /// Changes whenever a saved folder sort may have changed, for bindings that resolve one.
    pub(in crate::ui) fn folder_sorts_revision(&self) -> u64 {
        self.folder_sorts_revision.get()
    }

    fn remembered_key(&self, location: &Location) -> Option<FolderKey> {
        if !self.remember_folder_views() {
            return None;
        }
        key_for_location(location)
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
            let mut views = store.views.borrow_mut();
            (opened && views.touch(&key), views.view(&key).sort)
        };
        if touched {
            self.schedule_folder_views_write(store, USE_SAVE_DELAY);
        }
        match sort {
            Some((sort_key, sort_direction)) => FolderSort::Saved(sort_key, sort_direction),
            None => FolderSort::Default,
        }
    }

    /// A sort equal to the default is not stored, so the folder follows later defaults.
    pub(in crate::ui) fn set_folder_sort(
        &self,
        location: &Location,
        sort_key: SortKey,
        sort_direction: SortDirection,
    ) {
        if sort_key.stored_name().is_none() {
            return;
        }
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        let sort = Some((sort_key, sort_direction)).filter(|sort| *sort != self.default_sort());
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
        if !store_sort(
            &mut self.preferences.borrow_mut(),
            (sort_key, sort_direction),
        ) {
            return;
        }
        self.save_preferences();
        if self.remember_folder_views() {
            self.change_folder_views(Affects::Sorts, |views| {
                views.prune_default_sort((sort_key, sort_direction))
            });
        }
    }

    pub(in crate::ui) fn icons_size_for(&self, location: Option<&Location>) -> i32 {
        location
            .and_then(|location| self.remembered_key(location))
            .and_then(|key| {
                self.folder_view_store()
                    .views
                    .borrow()
                    .view(&key)
                    .icons_size
            })
            .unwrap_or_else(|| self.icons_thumbnail_size())
    }

    /// A folder that is not remembered keeps a changed size only while it is shown.
    pub(in crate::ui) fn set_folder_icons_size(&self, location: Option<&Location>, size: i32) {
        let size = size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        if !self.remember_folder_views() {
            self.set_icons_thumbnail_size(size);
            return;
        }
        let Some(key) = location.and_then(key_for_location) else {
            return;
        };
        let icons_size = (size != self.icons_thumbnail_size()).then_some(size);
        self.change_folder_views(Affects::IconsSizesOnly, |views| {
            views.update(&key, |view| view.icons_size = icons_size)
        });
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
        let size = size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        self.set_icons_thumbnail_size(size);
        if self.remember_folder_views() {
            self.change_folder_views(Affects::IconsSizesOnly, |views| {
                views.prune_default_icons_size(size)
            });
        }
    }

    pub(in crate::ui) fn has_folder_views(&self) -> bool {
        !self.folder_view_store().views.borrow().is_empty()
    }

    /// Also clears what another Strata process saved and this one has not merged yet.
    pub(in crate::ui) fn forget_folder_views(&self) {
        let merged = self
            .folder_view_store()
            .merge_saved_elsewhere(&folder_views_path())
            .unwrap_or(false);
        self.change_folder_views(Affects::Sorts, |views| views.clear() || merged);
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
        trash_identity: Option<TrashedOriginal>,
    ) {
        let Some(key) = key_for_location(location) else {
            return;
        };
        let store = self.folder_view_store();
        if let Some(identity) = trash_identity {
            store.trashed.borrow_mut().forget_identity(identity);
        }
        let taken = store.views.borrow_mut().take_within(&key);
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

    pub(in crate::ui) fn restore_folder_views(&self, location: &Location) {
        let (Some(store), Some(root)) = (self.folder_views.get(), location.native_path()) else {
            return;
        };
        let taken = store.trashed.borrow_mut().take_restored(root);
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
        if !self.folder_view_store().views.borrow().has_within(&key)
            || !folder_was_deleted(path, &key, holds_mount_points)
        {
            return;
        }
        self.change_folder_views(Affects::Sorts, |views| views.relocate(&key, None, false));
    }

    /// Another Strata process, such as the portal chooser, can change the
    /// defaults that folder values are stored relative to.
    fn adopt_defaults_saved_elsewhere(&self) {
        let stamp = file_stamp(&settings_path());
        if self.load_failure.is_some() || stamp == self.settings_synced.get() {
            return;
        }
        let Ok(saved) = read_preferences() else {
            return;
        };
        self.settings_synced.set(stamp);
        let size = saved
            .icons_thumbnail_size
            .clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        {
            let mut preferences = self.preferences.borrow_mut();
            if preferences.sort_key == saved.sort_key
                && preferences.sort_direction == saved.sort_direction
                && preferences.icons_thumbnail_size == size
            {
                return;
            }
            preferences.sort_key = saved.sort_key;
            preferences.sort_direction = saved.sort_direction;
            preferences.icons_thumbnail_size = size;
        }
        if self.changes.record(&self.preferences.borrow()) {
            self.changes.notify(self);
        }
    }

    fn merge_folder_views_saved_elsewhere(&self) {
        if let Some(store) = self.folder_views.get()
            && store
                .merge_saved_elsewhere(&folder_views_path())
                .unwrap_or(false)
        {
            self.publish_folder_views(Affects::Sorts);
        }
    }

    pub(in crate::ui) fn flush_folder_views(&self) {
        if let Some(store) = self.folder_views.get()
            && store.views.borrow().has_unsaved()
        {
            self.write_folder_views(store);
        }
    }

    fn change_folder_views(&self, affects: Affects, change: impl FnOnce(&mut FolderViews) -> bool) {
        let store = self.folder_view_store();
        if change(&mut store.views.borrow_mut()) {
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
        if store.load_failure.is_some() {
            self.write_folder_views(store);
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
                manager.write_folder_views(store);
            }
        });
        *timer = Some((source, delay));
    }

    fn write_folder_views(&self, store: &FolderViewStore) {
        if let Some((timer, _)) = store.save_timer.borrow_mut().take() {
            timer.remove();
        }
        let path = folder_views_path();
        if let Some(error) = &store.load_failure {
            self.folder_save_notices.unreadable_at_startup(&path, error);
            return;
        }
        let merged = match store.merge_saved_elsewhere(&path) {
            Ok(merged) => merged,
            Err(error) => {
                self.folder_save_notices.write_failed(&path, &error);
                return;
            }
        };
        let contents = store.views.borrow().to_toml();
        let result = contents.and_then(|contents| {
            self.folder_save_notices
                .write(&path, &contents, crate::storage::atomic_write)
        });
        match result {
            Ok(()) => {
                store.views.borrow_mut().mark_saved();
                store.synced.set(file_stamp(&path));
            }
            Err(error) => self.folder_save_notices.write_failed(&path, &error),
        }
        if merged {
            self.publish_folder_views(Affects::Sorts);
        }
    }
}
