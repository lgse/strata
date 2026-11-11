// SPDX-License-Identifier: MIT

//! The saved settings: what `settings.toml` held when the process started, and the few
//! values this interface writes back.
//!
//! The GTK application's `PreferenceManager` (`src/ui/preferences.rs`) does the same over
//! the same store, `strata_desktop::settings`, with its bindings and notices on top. A save
//! replaces only the keys this process changed, so another Strata process's changes stay.

use std::collections::BTreeSet;

use strata_core::{
    model::ViewPreferences,
    storage::writer::{self, Policy},
};
use strata_desktop::settings::{
    Preferences, SettingKey, merged_settings, read_loaded_preferences, record_changes,
    settings_path, store_sort,
};

pub(crate) struct Saved {
    preferences: Preferences,
    /// The file was there and could not be read: it is the user's to repair, and nothing is
    /// saved over it.
    unreadable: bool,
    /// What the file was last asked to hold, and the keys changed since the process started.
    queued: toml::Table,
    changed: BTreeSet<SettingKey>,
}

fn table(preferences: &Preferences) -> toml::Table {
    toml::Table::try_from(preferences).unwrap_or_default()
}

impl Saved {
    /// Reads the file, salvaged entry by entry and brought within range, and makes its
    /// language the process's.
    pub(crate) fn load() -> Self {
        let (mut preferences, failure, _salvaged) = read_loaded_preferences();
        preferences.normalize_loaded();
        rust_i18n::set_locale(preferences.language.locale());
        Self {
            queued: table(&preferences),
            preferences,
            unreadable: failure.is_some(),
            changed: BTreeSet::new(),
        }
    }

    pub(crate) fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// The browser's sort or hidden-files choice changed: `set_sort_preferences` in
    /// `src/ui/preferences/layout_settings.rs`.
    pub(crate) fn store_view(&mut self, view: ViewPreferences) {
        self.preferences.show_hidden = view.show_hidden;
        self.preferences.folders_first = view.folders_first;
        store_sort(&mut self.preferences, view.sort_options());
        self.save();
    }

    /// Queues a save of what changed, off the interface thread.
    fn save(&mut self) {
        let snapshot = table(&self.preferences);
        let before = self.changed.len();
        record_changes(&mut self.changed, &self.queued, &snapshot);
        if self.unreadable || self.changed.len() == before && snapshot == self.queued {
            return;
        }
        self.queued = snapshot.clone();
        let keys = self.changed.clone();
        let path = settings_path();
        let target = path.clone();
        let saved = writer::save_locked(path.clone(), Policy::Config, move || {
            merged_settings(&target, &snapshot, &keys)
        });
        // The GTK window shows "Settings can't be saved"; this interface has no notice yet.
        glib::MainContext::default().spawn_local(async move {
            if let Ok(Err(error)) = saved.await {
                tracing::warn!(%error, path = %path.display(), "unable to save the settings");
            }
        });
    }
}
