// SPDX-License-Identifier: MIT

//! Saving `settings.toml`: only what this process changed, off the interface thread.

use super::*;

impl PreferenceManager {
    pub(super) fn save_preferences(&self) {
        let changed = self.changes.record(&self.preferences.borrow());
        if !changed && !self.persistence_dirty.get() {
            return;
        }
        let path = settings_path();
        if let Some(error) = &self.load_failure {
            if changed {
                self.changes.notify(self);
                self.save_notices.unreadable_at_startup(&path, error);
            }
            return;
        }
        self.persistence_dirty.set(true);
        let snapshot = toml::Table::try_from(&*self.preferences.borrow()).map_err(io::Error::other);
        if let Ok(snapshot) = &snapshot {
            self.queue_settings_save(path.clone(), snapshot.clone());
        }
        if changed {
            self.changes.notify(self);
        }
        // Reported after bindings apply, and whether or not anything changed,
        // because a retry with an unchanged value can fail too.
        if let Err(error) = snapshot {
            self.save_notices.write_failed(&path, &error);
        }
    }

    fn queue_settings_save(&self, path: PathBuf, snapshot: toml::Table) {
        let generation = self.saves_requested.get() + 1;
        self.saves_requested.set(generation);
        let keys = {
            let mut last = self.last_queued.borrow_mut();
            let mut dirty = self.dirty_keys.borrow_mut();
            record_changes(&mut dirty, &last, &snapshot);
            *last = snapshot.clone();
            // Never serialized, so a migration cannot show up as a change.
            if self.preferences.borrow().legacy_omarchy_variant.is_none() {
                dirty.insert(("omarchy_variant".to_owned(), None));
            }
            dirty.extend(RETIRED_SETTINGS.map(|key| (key.to_owned(), None)));
            dirty.clone()
        };
        let target = path.clone();
        let saved = writer::save_locked(path.clone(), Policy::Config, move || {
            merged_settings(&target, &snapshot, &keys)
        });
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = saved
                .await
                .unwrap_or_else(|_| Err(io::Error::from_raw_os_error(libc::ECANCELED)));
            if let Some(manager) = this.upgrade() {
                manager.settings_saved(generation, &path, result);
            }
        });
    }

    fn settings_saved(&self, generation: u64, path: &Path, result: io::Result<()>) {
        if generation != self.saves_requested.get() {
            return;
        }
        match result {
            Ok(()) => {
                self.persistence_dirty.set(false);
                self.dirty_keys.borrow_mut().clear();
                self.save_notices.saved();
            }
            Err(error) => self.save_notices.write_failed(path, &error),
        }
    }

    /// Whether a save not yet written, or one that failed, would replace `key` on disk.
    pub(super) fn setting_unsaved(&self, key: &str) -> bool {
        self.dirty_keys
            .borrow()
            .iter()
            .any(|(dirty, _)| dirty == key)
    }
}
