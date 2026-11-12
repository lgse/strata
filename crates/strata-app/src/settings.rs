// SPDX-License-Identifier: MIT

//! The saved settings: what `settings.toml` held when the process started, and what this
//! interface writes back.
//!
//! The values are a [`Preferences`] and the bookkeeping of saving them is
//! `strata_desktop::settings::Saving`, the same one the GTK application's
//! `PreferenceManager` (`src/ui/preferences.rs`) wraps with its bindings and notices. So a
//! save here writes what a save there writes: the keys this process changed since a save
//! last succeeded, with what loading repaired, and another Strata process's other keys stay.
//!
//! A part reads the settings with `shell::settings` and changes one with
//! `shell::saved(app).change(…)`.

use strata_core::model::ViewPreferences;
use strata_desktop::settings::{
    Commit, FinishedSave, Preferences, SaveOutcome, SaveProblem, Saving, store_sort,
};

use crate::{App, slots::Later};

pub(crate) struct Saved {
    preferences: Preferences,
    saving: Saving,
    /// The window's queue, where a finished save tells the bookkeeping its result. Without
    /// it a save is still written, and its keys are written again with the next.
    later: Option<Later<App>>,
}

impl Saved {
    /// Reads the file, salvaged entry by entry and brought within range, and makes its
    /// language the process's.
    pub(crate) fn load() -> Self {
        let (preferences, saving) = strata_desktop::settings::load();
        rust_i18n::set_locale(saving.startup_locale());
        Self {
            preferences,
            saving,
            later: None,
        }
    }

    /// Gives the settings the window's queue, once the window has one.
    pub(crate) fn report_to(&mut self, later: Later<App>) {
        self.later = Some(later);
    }

    pub(crate) fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// The browser's sort or hidden-files choice changed: `set_sort_preferences` in
    /// `src/ui/preferences/layout_settings.rs`.
    pub(crate) fn store_view(&mut self, view: ViewPreferences) {
        self.change(|preferences| {
            preferences.show_hidden = view.show_hidden;
            preferences.folders_first = view.folders_first;
            store_sort(preferences, view.sort_options());
        });
    }

    /// Changes the saved settings and queues a save of what changed, off the interface
    /// thread. A file that could not be read at start-up is the user's to repair, and
    /// nothing is saved over it.
    pub(crate) fn change(&mut self, change: impl FnOnce(&mut Preferences)) {
        change(&mut self.preferences);
        let Commit { save, problem, .. } = self.saving.commit(&self.preferences);
        // The GTK window shows "Settings can't be saved"; this interface has no notice yet.
        if let Some(SaveProblem::Snapshot { path, error }) = problem {
            tracing::warn!(%error, path = %path.display(), "unable to save the settings");
        }
        let Some(save) = save else {
            return;
        };
        let later = self.later.clone();
        // GLib dispatches the end of the save, so it only queues: the bookkeeping hears of it
        // in the next `poll`.
        glib::MainContext::default().spawn_local(async move {
            let finished = save.finished().await;
            if let Some(later) = later {
                later.then(move |app: &mut App, _| {
                    crate::shell::saved(app).saved(finished);
                    false
                });
            }
        });
    }

    fn saved(&mut self, finished: FinishedSave) {
        if let SaveOutcome::Failed { path, error } = self.saving.saved(finished) {
            tracing::warn!(%error, path = %path.display(), "unable to save the settings");
        }
    }
}
