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
//! A part reaches the settings through `shell::saved(app)`: `preferences()` to read them as
//! they are now, and `change(…)` to change one and have it saved. `slots.rs` has the rules.

use std::{io, path::Path};

use strata_core::model::ViewPreferences;
use strata_desktop::settings::{
    ADOPTED_DEFAULT_KEYS, Commit, FinishedSave, Preferences, SaveOutcome, SaveProblem, Saving,
    ThemeAvailability,
    save_notice::{NoticeProblem, SaveSubject, save_notice_summary, save_notice_text},
    store_sort,
};
use strata_fs::services::io_error_detail;

use crate::{App, slots::Later};

/// A save or startup-read notice waiting to be shown once per failure streak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSaveNotice {
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) message: String,
}

#[derive(Default)]
struct NoticeStreaks {
    settings_shown: bool,
    folder_views_shown: bool,
    pending: Option<PendingSaveNotice>,
}

struct NoticeFailure<'a> {
    subject: SaveSubject,
    problem: NoticeProblem,
    path: &'a Path,
    error: &'a io::Error,
}

impl NoticeStreaks {
    fn arm(&mut self, subject: SaveSubject) {
        match subject {
            SaveSubject::Settings => self.settings_shown = false,
            SaveSubject::FolderViews => self.folder_views_shown = false,
        }
    }

    fn raise(&mut self, failure: NoticeFailure<'_>) -> bool {
        let shown = match failure.subject {
            SaveSubject::Settings => &mut self.settings_shown,
            SaveSubject::FolderViews => &mut self.folder_views_shown,
        };
        if *shown {
            return false;
        }
        *shown = true;
        let reason = io_error_detail(failure.error);
        let (title, message) =
            save_notice_text(failure.subject, failure.problem, failure.path, &reason);
        self.pending = Some(PendingSaveNotice {
            title,
            summary: save_notice_summary(),
            message,
        });
        true
    }
}

/// One for the process, made at start-up and owned by the window: two would lose each
/// other's saves (`strata_desktop::settings::Saving`).
pub(crate) struct Saved {
    preferences: Preferences,
    saving: Saving,
    /// The window's queue, where a finished save tells the bookkeeping its result. Without
    /// it a save is still written, and its keys are written again with the next.
    later: Option<Later<App>>,
    notices: NoticeStreaks,
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
            notices: NoticeStreaks::default(),
        }
    }

    /// Gives the settings the window's queue, once the window has one.
    pub(crate) fn report_to(&mut self, later: Later<App>) {
        self.later = Some(later);
    }

    pub(crate) fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    pub(crate) fn language_restart_required(&self) -> bool {
        self.preferences.language.locale() != self.saving.startup_locale()
    }

    pub(crate) fn interface_renderer_restart_required(&self) -> bool {
        self.preferences.interface_renderer != self.saving.startup_interface_renderer()
    }

    pub(crate) fn can_adopt_defaults(&self) -> bool {
        self.saving.load_failure().is_none()
    }

    /// Repairs loaded theme and mode choices at startup without counting a user change.
    pub(crate) fn normalize_themes(
        &mut self,
        theme_is_known: impl Fn(&str) -> bool,
        available: ThemeAvailability,
    ) {
        self.preferences
            .normalize_theme_selection(theme_is_known, available);
        self.saving.rebase(&self.preferences);
    }

    /// Adopts default sort and icon-size keys another Strata process wrote, provided none of
    /// those keys have unsaved edits in this process.
    pub(crate) fn adopt_defaults_from_disk(&mut self, saved: Preferences) -> bool {
        if self.saving.load_failure().is_some()
            || ADOPTED_DEFAULT_KEYS
                .iter()
                .any(|key| self.saving.setting_unsaved(key))
        {
            return false;
        }
        if !self.preferences.adopt_defaults(saved) {
            return false;
        }
        self.saving
            .record_adopted(&self.preferences, &ADOPTED_DEFAULT_KEYS);
        true
    }

    pub(crate) fn take_notice(&mut self) -> Option<PendingSaveNotice> {
        self.notices.pending.take()
    }

    pub(crate) fn folder_views_saved(&mut self) {
        self.notices.arm(SaveSubject::FolderViews);
    }

    pub(crate) fn report_folder_views_problem(
        &mut self,
        problem: NoticeProblem,
        path: &Path,
        error: &io::Error,
    ) -> bool {
        self.notices.raise(NoticeFailure {
            subject: SaveSubject::FolderViews,
            problem,
            path,
            error,
        })
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

    /// Mutates `Preferences` in place without committing yet, for batches that commit once.
    pub(crate) fn mutate(&mut self, change: impl FnOnce(&mut Preferences) -> bool) -> bool {
        change(&mut self.preferences)
    }

    pub(crate) fn mutate_in_place<R>(&mut self, change: impl FnOnce(&mut Preferences) -> R) -> R {
        change(&mut self.preferences)
    }

    /// Commits whatever changed in `Preferences` since the last save and queues the write off
    /// the interface thread. A file that could not be read at start-up is the user's to repair,
    /// and nothing is saved over it.
    pub(crate) fn commit(&mut self) {
        let Commit { save, problem, .. } = self.saving.commit(&self.preferences);
        match problem {
            Some(SaveProblem::UnreadableAtStartup { path }) => {
                if let Some(error) = self.saving.load_failure() {
                    self.notices.raise(NoticeFailure {
                        subject: SaveSubject::Settings,
                        problem: NoticeProblem::UnreadableAtStartup,
                        path: &path,
                        error,
                    });
                }
            }
            Some(SaveProblem::Snapshot { path, error }) => {
                tracing::warn!(%error, path = %path.display(), "unable to save the settings");
                self.notices.raise(NoticeFailure {
                    subject: SaveSubject::Settings,
                    problem: NoticeProblem::WriteFailed,
                    path: &path,
                    error: &error,
                });
            }
            None => {}
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
                later.then(move |app: &mut App, _| crate::shell::saved(app).saved(finished));
            }
        });
    }

    /// Changes the saved settings and queues a save of what changed, off the interface
    /// thread. Turning `restore_tabs` off also removes `tabs.toml` (`set_restore_tabs` in
    /// `src/ui/preferences/file_settings.rs`). A file that could not be read at start-up is
    /// the user's to repair, and nothing is saved over it.
    pub(crate) fn change(&mut self, change: impl FnOnce(&mut Preferences)) {
        let was_restore_tabs = self.preferences.restore_tabs;
        change(&mut self.preferences);
        if was_restore_tabs && !self.preferences.restore_tabs {
            strata_desktop::tabs_session::remove();
        }
        self.commit();
    }

    fn saved(&mut self, finished: FinishedSave) -> bool {
        match self.saving.saved(finished) {
            SaveOutcome::Saved => {
                self.notices.arm(SaveSubject::Settings);
                false
            }
            SaveOutcome::Failed { path, error } => {
                tracing::warn!(%error, path = %path.display(), "unable to save the settings");
                self.notices.raise(NoticeFailure {
                    subject: SaveSubject::Settings,
                    problem: NoticeProblem::WriteFailed,
                    path: &path,
                    error: &error,
                })
            }
            SaveOutcome::Superseded => false,
        }
    }
}
