// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use gtk::glib;

use crate::{
    adapters::{Watched, probes},
    model::{FolderSort, Location, SortDirection, SortKey},
    services::{ItemIdentity, LoadHandle},
};

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, PreferenceManager, SHARED_MANAGER,
    file_stamp,
    folder_views::{FolderKey, FolderViews, TakenViews, key_for_location},
    parse_preferences, read_state_file, settings_path, store_sort,
    trashed::TrashedValues,
};
use crate::storage::writer;

const FILE_NAME: &str = "folder-views.toml";
/// Coalesces bursts of changes, such as a thumbnail-size drag, into one write
/// that stays off the click that made them.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Opening folders only refreshes use times, which can wait for other changes.
const USE_SAVE_DELAY: Duration = Duration::from_secs(60);
/// Another process saves in milliseconds; one stopped mid-save must not hold up exit.
const EXIT_LOCK_WAIT: Duration = Duration::from_secs(1);

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
        let path = folder_views_path();
        writer::note(&path, file_stamp(&path));
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

pub(in crate::ui) fn folder_views_path() -> PathBuf {
    crate::storage::state_directory().join(FILE_NAME)
}

/// Keeps other Strata processes from saving between this one's merge and its
/// rename, which would drop their changes. Locks the directory, because every
/// save replaces the file.
pub(super) fn lock_saves(path: &Path, wait: Duration) -> io::Result<fs::File> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    fs::create_dir_all(directory)?;
    let lock = fs::File::open(directory)?;
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(lock),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
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
        self.preferences.borrow_mut().remember_folder_views = enabled;
        if enabled {
            let (sort, size) = (self.default_sort(), self.icons_thumbnail_size());
            self.change_folder_views(Affects::Sorts, |views| {
                views.prune_default_sort(sort) | views.prune_default_icons_size(size)
            });
        }
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
        if !self.folder_view_store().views.borrow().may_forget(&key) {
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
        if self.load_failure.is_some() {
            return;
        }
        let saved = writer::reread(settings_path(), parse_preferences);
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(Ok(Some(saved))), Some(manager)) = (saved.await, this.upgrade())
                && !["sort_key", "sort_direction", "icons_thumbnail_size"]
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
                && preferences.icons_thumbnail_size == size
            {
                return;
            }
            preferences.sort_key = saved.sort_key;
            preferences.sort_direction = saved.sort_direction;
            preferences.icons_thumbnail_size = size;
            if let Ok(current) = toml::Table::try_from(&*preferences) {
                let mut last = self.last_queued.borrow_mut();
                let mut dirty = self.dirty_keys.borrow_mut();
                for key in ["sort_key", "sort_direction", "icons_thumbnail_size"] {
                    match current.get(key) {
                        Some(value) => last.insert(key.to_owned(), value.clone()),
                        None => last.remove(key),
                    };
                    dirty.remove(&(key.to_owned(), None));
                }
            }
        }
        if self.changes.record(&self.preferences.borrow()) {
            self.changes.notify(self);
        }
    }

    fn merge_folder_views_saved_elsewhere(&self) {
        if self.folder_views.get().is_none() {
            return;
        }
        let saved = writer::reread(folder_views_path(), |contents| {
            FolderViews::parse(contents).map(|(views, _)| views)
        });
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(Ok(Some(saved))), Some(manager)) = (saved.await, this.upgrade()) {
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
        let merged = store.views.borrow().merged_over(saved);
        let changed = !merged.same_views(&store.views.borrow());
        store.views.replace(merged);
        if changed {
            self.publish_folder_views(Affects::Sorts);
        }
    }

    pub(in crate::ui) fn flush_folder_views(&self) {
        if let Some(store) = self.folder_views.get()
            && store.views.borrow().has_unsaved()
        {
            self.write_folder_views(store, true);
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
        let path = folder_views_path();
        if let Some(error) = &store.load_failure {
            self.folder_save_notices.unreadable_at_startup(&path, error);
            return;
        }
        let snapshot = store.views.borrow().clone();
        let edits = snapshot.edits();
        let saving = path.clone();
        let saved = writer::run(move || save_folder_views(&saving, snapshot, exit));
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let saved = saved.await.unwrap_or_else(|_| {
                FolderViewsSave::Failed(io::Error::from_raw_os_error(libc::ECANCELED))
            });
            if let Some(manager) = this.upgrade() {
                manager.folder_views_saved(&path, edits, saved);
            }
        });
    }

    fn folder_views_saved(&self, path: &Path, edits: u64, saved: FolderViewsSave) {
        let Some(store) = self.folder_views.get() else {
            return;
        };
        match saved {
            FolderViewsSave::Busy => self.schedule_folder_views_write(store, SAVE_DELAY),
            FolderViewsSave::Failed(error) => self.folder_save_notices.write_failed(path, &error),
            FolderViewsSave::Saved(merged) => {
                self.folder_save_notices.saved();
                if let Some(merged) = merged {
                    self.adopt_folder_views(merged);
                }
                store.views.borrow_mut().mark_saved_through(edits);
            }
        }
    }
}

enum FolderViewsSave {
    Busy,
    Saved(Option<FolderViews>),
    Failed(io::Error),
}

/// An unreadable file, or one from a newer version, must not be overwritten.
fn save_folder_views(path: &Path, snapshot: FolderViews, exit: bool) -> FolderViewsSave {
    let lock = lock_saves(path, if exit { EXIT_LOCK_WAIT } else { Duration::ZERO });
    if let Err(error) = &lock {
        if error.kind() == io::ErrorKind::WouldBlock && !exit {
            return FolderViewsSave::Busy;
        }
        tracing::debug!(%error, path = %path.display(), "saving folder settings unlocked");
    }
    let saved = match read_state_file(path).and_then(|contents| {
        contents
            .map(|contents| FolderViews::parse(&contents).map(|(views, _)| views))
            .transpose()
    }) {
        Ok(saved) => saved,
        Err(error) => return FolderViewsSave::Failed(error),
    };
    let (written, changed) = match saved {
        Some(saved) => {
            let merged = snapshot.merged_over(saved);
            let changed = !merged.same_views(&snapshot);
            (merged, changed)
        }
        None => (snapshot, false),
    };
    let result = written.to_toml().and_then(|contents| {
        fs::create_dir_all(path.parent().unwrap_or(path))?;
        crate::storage::atomic_write(path, contents.as_bytes())
    });
    if result.is_ok() {
        writer::note(path, file_stamp(path));
    }
    drop(lock);
    match result {
        Ok(()) => FolderViewsSave::Saved(changed.then_some(written)),
        Err(error) => FolderViewsSave::Failed(error),
    }
}
