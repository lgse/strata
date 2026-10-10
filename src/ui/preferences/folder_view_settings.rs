// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

use gtk::glib;

use crate::model::{FolderSort, Location, SortDirection, SortKey};

use super::{
    MAX_ICONS_THUMBNAIL_SIZE, MIN_ICONS_THUMBNAIL_SIZE, PreferenceManager, SHARED_MANAGER,
    folder_views::{FolderKey, FolderViews, key_for_location, stored_direction, stored_sort_key},
    save_notice::SaveProblem,
};

const FILE_NAME: &str = "folder-views.toml";
/// Coalesces a thumbnail-size drag into one write.
const SAVE_DELAY: Duration = Duration::from_millis(500);

pub(super) struct FolderViewStore {
    views: RefCell<FolderViews>,
    /// Why saving is off for this session: the file existed but could not be read.
    load_failure: Option<io::Error>,
    /// Changes or use times not written yet.
    dirty: Cell<bool>,
    save_timer: RefCell<Option<glib::SourceId>>,
    /// The file as this process last read or wrote it; anything else on disk was
    /// written by another Strata process and is merged before saving.
    synced: Cell<Option<FileStamp>>,
}

type FileStamp = (u64, i64, i64, u64);

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some((
        metadata.ino(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.len(),
    ))
}

impl FolderViewStore {
    fn load() -> Self {
        let path = folder_views_path();
        let synced = file_stamp(&path);
        let (views, load_failure) = match fs::read_to_string(&path) {
            Ok(contents) => match FolderViews::parse(&contents) {
                Ok((views, repaired)) => {
                    if repaired {
                        tracing::warn!(path = %path.display(),
                            "folder settings file has invalid entries; keeping the valid ones");
                    }
                    (views, None)
                }
                Err(error) => (FolderViews::default(), Some(error)),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => (FolderViews::default(), None),
            Err(error) => (FolderViews::default(), Some(error)),
        };
        if let Some(error) = &load_failure {
            tracing::warn!(%error, path = %path.display(),
                "unable to load folder settings; using none without saving; fix the file and restart Strata");
        }
        Self {
            views: RefCell::new(views),
            load_failure,
            dirty: Cell::new(false),
            save_timer: RefCell::new(None),
            synced: Cell::new(synced),
        }
    }

    /// Takes in what another process saved since this one last read or wrote
    /// the file, and returns whether that changed any folder's values.
    fn merge_saved_elsewhere(&self, path: &Path) -> bool {
        let Some(stamp) = file_stamp(path).filter(|stamp| Some(*stamp) != self.synced.get()) else {
            return false;
        };
        let Some((saved, _)) = fs::read_to_string(path)
            .ok()
            .and_then(|contents| FolderViews::parse(&contents).ok())
        else {
            return false;
        };
        let merged = self.views.borrow().merged_over(saved);
        let changed = !merged.same_views(&self.views.borrow());
        self.views.replace(merged);
        self.synced.set(Some(stamp));
        changed
    }
}

#[derive(Clone, Copy)]
enum Save {
    Now,
    Soon,
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

    pub(in crate::ui) fn set_remember_folder_views(&self, enabled: bool) {
        self.preferences.borrow_mut().remember_folder_views = enabled;
        self.save_preferences();
    }

    /// Changes whenever a saved folder value changes, for bindings that resolve one.
    pub(in crate::ui) fn folder_views_revision(&self) -> u64 {
        self.folder_views_revision.get()
    }

    fn remembered_key(&self, location: &Location) -> Option<FolderKey> {
        if !self.remember_folder_views() {
            return None;
        }
        key_for_location(location)
    }

    pub(in crate::ui) fn folder_sort(&self, location: &Location) -> FolderSort {
        let Some(key) = self.remembered_key(location) else {
            return FolderSort::Unremembered;
        };
        let store = self.folder_view_store();
        let mut views = store.views.borrow_mut();
        if views.touch(&key) {
            store.dirty.set(true);
        }
        match views.view(&key).sort {
            Some((sort_key, sort_direction)) => FolderSort::Saved(sort_key, sort_direction),
            None => FolderSort::Default,
        }
    }

    pub(in crate::ui) fn has_folder_sort(&self, location: &Location) -> bool {
        self.remembered_key(location).is_some_and(|key| {
            self.folder_view_store()
                .views
                .borrow()
                .view(&key)
                .sort
                .is_some()
        })
    }

    /// A sort equal to the default is not stored, so the folder follows later defaults.
    pub(in crate::ui) fn set_folder_sort(
        &self,
        location: &Location,
        sort_key: SortKey,
        sort_direction: SortDirection,
    ) {
        if stored_sort_key(sort_key).is_none() {
            return;
        }
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        let sort = Some((sort_key, sort_direction)).filter(|sort| *sort != self.default_sort());
        self.change_folder_views(Save::Now, |views| {
            views.update(&key, |view| view.sort = sort)
        });
    }

    pub(in crate::ui) fn reset_folder_sort(&self, location: &Location) {
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        self.change_folder_views(Save::Now, |views| {
            views.update(&key, |view| view.sort = None)
        });
    }

    pub(in crate::ui) fn default_sort(&self) -> (SortKey, SortDirection) {
        let preferences = self.sort_preferences();
        (preferences.sort_key, preferences.sort_direction)
    }

    pub(in crate::ui) fn default_sort_key(&self) -> SortKey {
        self.default_sort().0
    }

    pub(in crate::ui) fn set_default_sort_key(&self, sort_key: SortKey) {
        self.set_default_sort(sort_key, self.default_sort().1);
    }

    pub(in crate::ui) fn default_sort_direction(&self) -> SortDirection {
        self.default_sort().1
    }

    pub(in crate::ui) fn set_default_sort_direction(&self, sort_direction: SortDirection) {
        self.set_default_sort(self.default_sort().0, sort_direction);
    }

    /// Folders whose saved sort now matches the default stop storing it.
    pub(in crate::ui) fn set_default_sort(&self, sort_key: SortKey, sort_direction: SortDirection) {
        let Some(stored_key) = stored_sort_key(sort_key) else {
            return;
        };
        {
            let mut preferences = self.preferences.borrow_mut();
            preferences.sort_key = stored_key.to_owned();
            preferences.sort_direction = stored_direction(sort_direction).to_owned();
        }
        self.save_preferences();
        let sort = Some((sort_key, sort_direction));
        self.change_folder_views(Save::Now, |views| {
            views.update_all(|view| {
                if view.sort == sort {
                    view.sort = None;
                }
            })
        });
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

    /// Without per-folder settings the size is the default. A folder that is not
    /// remembered keeps a changed size only while it is shown.
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
        self.change_folder_views(Save::Soon, |views| {
            views.update(&key, |view| view.icons_size = icons_size)
        });
    }

    pub(in crate::ui) fn reset_folder_icons_size(&self, location: &Location) {
        let Some(key) = self.remembered_key(location) else {
            return;
        };
        self.change_folder_views(Save::Now, |views| {
            views.update(&key, |view| view.icons_size = None)
        });
    }

    /// Folders whose saved size now matches the default stop storing it.
    pub(in crate::ui) fn set_default_icons_size(&self, size: i32) {
        let size = size.clamp(MIN_ICONS_THUMBNAIL_SIZE, MAX_ICONS_THUMBNAIL_SIZE);
        self.set_icons_thumbnail_size(size);
        self.change_folder_views(Save::Now, |views| {
            views.update_all(|view| {
                if view.icons_size == Some(size) {
                    view.icons_size = None;
                }
            })
        });
    }

    pub(in crate::ui) fn has_folder_views(&self) -> bool {
        !self.folder_view_store().views.borrow().is_empty()
    }

    pub(in crate::ui) fn forget_folder_views(&self) {
        self.change_folder_views(Save::Now, FolderViews::clear);
    }

    /// Saved values follow a folder renamed or moved within Strata, whether or not
    /// per-folder settings are in use, and are dropped when it moves somewhere
    /// that is not remembered.
    pub(in crate::ui) fn relocate_folder_views(&self, moves: &[(Location, Location)]) {
        let moves: Vec<(FolderKey, Option<FolderKey>)> = moves
            .iter()
            .filter_map(|(from, to)| Some((key_for_location(from)?, key_for_location(to))))
            .collect();
        if moves.is_empty() {
            return;
        }
        self.change_folder_views(Save::Now, |views| views.relocate(&moves));
    }

    pub(in crate::ui) fn remove_folder_views(&self, locations: &[Location]) {
        let removals: Vec<(FolderKey, Option<FolderKey>)> = locations
            .iter()
            .filter_map(|location| Some((key_for_location(location)?, None)))
            .collect();
        if removals.is_empty() {
            return;
        }
        self.change_folder_views(Save::Now, |views| views.relocate(&removals));
    }

    /// A local folder that failed to open because it no longer exists forgets
    /// its saved values and those of the folders inside it.
    pub(in crate::ui) fn forget_missing_folder(&self, location: &Location) {
        let Some(key) = key_for_location(location) else {
            return;
        };
        if self.folder_view_store().views.borrow().is_empty() {
            return;
        }
        let missing = location.native_path().is_some_and(|path| {
            fs::metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        });
        if missing {
            self.change_folder_views(Save::Now, |views| views.relocate(&[(key, None)]));
        }
    }

    pub(in crate::ui) fn flush_folder_views(&self) {
        if let Some(store) = self.folder_views.get()
            && store.dirty.get()
        {
            self.write_folder_views(store);
        }
    }

    fn change_folder_views(&self, save: Save, change: impl FnOnce(&mut FolderViews) -> bool) {
        let store = self.folder_view_store();
        if !change(&mut store.views.borrow_mut()) {
            return;
        }
        self.folder_views_revision
            .set(self.folder_views_revision.get().wrapping_add(1));
        store.dirty.set(true);
        match save {
            Save::Now => self.write_folder_views(store),
            Save::Soon => self.schedule_folder_views_write(store),
        }
        self.changes.notify(self);
    }

    fn schedule_folder_views_write(&self, store: &FolderViewStore) {
        if store.load_failure.is_some() {
            self.write_folder_views(store);
            return;
        }
        if store.save_timer.borrow().is_some() {
            return;
        }
        let source = glib::timeout_add_local_once(SAVE_DELAY, || {
            let Some(manager) = SHARED_MANAGER.with(|shared| shared.borrow().upgrade()) else {
                return;
            };
            if let Some(store) = manager.folder_views.get() {
                // This source is finishing, so it must not be removed again.
                store.save_timer.borrow_mut().take();
                manager.write_folder_views(store);
            }
        });
        store.save_timer.replace(Some(source));
    }

    fn write_folder_views(&self, store: &FolderViewStore) {
        if let Some(timer) = store.save_timer.borrow_mut().take() {
            timer.remove();
        }
        let path = folder_views_path();
        if let Some(error) = &store.load_failure {
            self.folder_save_notices.report(
                SaveProblem::UnreadableAtStartup,
                &path,
                &crate::services::io_error_detail(error),
            );
            return;
        }
        let merged = store.merge_saved_elsewhere(&path);
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let contents = store.views.borrow().to_toml()?;
            crate::storage::atomic_write(&path, contents.as_bytes())
        })();
        if merged {
            self.folder_views_revision
                .set(self.folder_views_revision.get().wrapping_add(1));
            self.changes.notify(self);
        }
        match result {
            Ok(()) => {
                store.views.borrow_mut().mark_saved();
                store.synced.set(file_stamp(&path));
                store.dirty.set(false);
                self.folder_save_notices.saved();
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "unable to save folder settings");
                self.folder_save_notices.report(
                    SaveProblem::WriteFailed,
                    &path,
                    &crate::services::io_error_detail(&error),
                );
            }
        }
    }
}
