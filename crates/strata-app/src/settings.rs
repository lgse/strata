// SPDX-License-Identifier: MIT

use std::{cell::RefCell, io, path::Path, rc::Rc};

use strata_core::model::ViewPreferences;
use strata_desktop::settings::{
    ADOPTED_DEFAULT_KEYS, Commit, FinishedSave, Preferences, RunePreferences, SaveOutcome,
    SaveProblem, Saving, ThemeAvailability, TransparencyPreferences,
    save_notice::{NoticeProblem, SaveSubject, save_notice_summary, save_notice_text},
    store_sort,
};
use strata_fs::services::io_error_detail;

use crate::{
    App,
    slots::{Later, WeakLater},
};

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

struct Store {
    preferences: Preferences,
    transparency: TransparencyPreferences,
    saving: Saving,
    /// Moves with every change any window makes.
    version: u64,
    /// The windows reading the store, each by its [`Saved::id`], to be told of a change.
    windows: Vec<(u64, WeakLater<App>)>,
    /// The id the next window's [`Saved`] takes.
    next: u64,
}

impl Store {
    fn tell_others(&mut self, changed_by: u64) {
        self.windows.retain(|(_, later)| later.is_live());
        for (id, later) in &self.windows {
            if *id == changed_by {
                continue;
            }
            if let Some(later) = later.upgrade() {
                later.then(crate::shell::follow_another_window);
            }
        }
        crate::host::windowed::wake_pending();
    }
}

pub(crate) struct Saved {
    store: Rc<RefCell<Store>>,
    /// This window's place among the store's windows.
    id: u64,
    /// The values as of `version`.
    preferences: Preferences,
    transparency: TransparencyPreferences,
    version: u64,
    later: Option<Later<App>>,
    notices: NoticeStreaks,
}

impl Drop for Saved {
    fn drop(&mut self) {
        if let Ok(mut store) = self.store.try_borrow_mut() {
            store.windows.retain(|(id, _)| *id != self.id);
        }
    }
}

impl Saved {
    pub(crate) fn load() -> Self {
        let (preferences, rune, saving) = strata_desktop::settings::load_rune();
        rust_i18n::set_locale(saving.startup_locale());
        let store = Store {
            preferences: preferences.clone(),
            transparency: rune.transparency,
            saving,
            version: 0,
            windows: Vec::new(),
            next: 1,
        };
        Self {
            store: Rc::new(RefCell::new(store)),
            id: 0,
            preferences,
            transparency: rune.transparency,
            version: 0,
            later: None,
            notices: NoticeStreaks::default(),
        }
    }

    /// The same settings for another window of the process, as they are now.
    pub(crate) fn share(&self) -> Self {
        let mut store = self.store.borrow_mut();
        let id = store.next;
        store.next += 1;
        Self {
            store: Rc::clone(&self.store),
            id,
            preferences: store.preferences.clone(),
            transparency: store.transparency,
            version: store.version,
            later: None,
            notices: NoticeStreaks::default(),
        }
    }

    pub(crate) fn report_to(&mut self, later: Later<App>) {
        let mut store = self.store.borrow_mut();
        store.windows.retain(|(id, _)| *id != self.id);
        store.windows.push((self.id, later.downgrade()));
        self.later = Some(later);
    }

    pub(crate) fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    pub(crate) fn transparency(&self) -> TransparencyPreferences {
        self.transparency
    }

    pub(crate) fn change_transparency(
        &mut self,
        change: impl FnOnce(&mut TransparencyPreferences),
    ) {
        self.follow();
        change(&mut self.transparency);
        self.transparency.normalize();
        self.commit();
    }

    /// How many windows the store would tell of a change, this one among them.
    #[cfg(test)]
    pub(crate) fn windows_told(&self) -> usize {
        let store = self.store.borrow();
        store
            .windows
            .iter()
            .filter(|(_, later)| later.is_live())
            .count()
    }

    pub(crate) fn follow(&mut self) -> bool {
        let store = self.store.borrow();
        if store.version == self.version {
            return false;
        }
        self.preferences = store.preferences.clone();
        self.transparency = store.transparency;
        self.version = store.version;
        true
    }

    /// Puts this window's values into the store, as the process's from now on.
    fn publish(&mut self, store: &mut Store) {
        store.preferences = self.preferences.clone();
        store.transparency = self.transparency;
        store.version += 1;
        self.version = store.version;
    }

    /// Puts this window's values into the store when they differ from the process's.
    fn publish_changed(&mut self, store: &mut Store) {
        if store.preferences != self.preferences || store.transparency != self.transparency {
            self.publish(store);
        }
    }

    pub(crate) fn language_restart_required(&self) -> bool {
        self.preferences.language.locale() != self.store.borrow().saving.startup_locale()
    }

    pub(crate) fn interface_renderer_restart_required(&self) -> bool {
        self.preferences.interface_renderer
            != self.store.borrow().saving.startup_interface_renderer()
    }

    pub(crate) fn can_adopt_defaults(&self) -> bool {
        self.store.borrow().saving.load_failure().is_none()
    }

    /// Repairs loaded theme and mode choices at startup without counting a user change.
    pub(crate) fn normalize_themes(
        &mut self,
        theme_is_known: impl Fn(&str) -> bool,
        available: ThemeAvailability,
    ) {
        self.follow();
        self.preferences
            .normalize_theme_selection(theme_is_known, available);
        let shared = Rc::clone(&self.store);
        let store = &mut *shared.borrow_mut();
        if store.preferences != self.preferences {
            self.publish(store);
            store.tell_others(self.id);
        }
        store.saving.rebase(&self.preferences);
    }

    pub(crate) fn adopt_defaults_from_disk(&mut self, saved: Preferences) -> bool {
        self.follow();
        let shared = Rc::clone(&self.store);
        let store = &mut *shared.borrow_mut();
        if store.saving.load_failure().is_some()
            || ADOPTED_DEFAULT_KEYS
                .iter()
                .any(|key| store.saving.setting_unsaved(key))
        {
            return false;
        }
        if !self.preferences.adopt_defaults(saved) {
            return false;
        }
        self.publish(store);
        store
            .saving
            .record_adopted(&self.preferences, &ADOPTED_DEFAULT_KEYS);
        store.tell_others(self.id);
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

    pub(crate) fn store_view(&mut self, view: ViewPreferences) {
        self.change(|preferences| {
            preferences.show_hidden = view.show_hidden;
            preferences.folders_first = view.folders_first;
            store_sort(preferences, view.sort_options());
        });
    }

    pub(crate) fn mutate(&mut self, change: impl FnOnce(&mut Preferences) -> bool) -> bool {
        self.follow();
        let changed = change(&mut self.preferences);
        if changed {
            let shared = Rc::clone(&self.store);
            self.publish(&mut shared.borrow_mut());
        }
        changed
    }

    pub(crate) fn mutate_in_place<R>(&mut self, change: impl FnOnce(&mut Preferences) -> R) -> R {
        self.follow();
        let result = change(&mut self.preferences);
        let shared = Rc::clone(&self.store);
        self.publish_changed(&mut shared.borrow_mut());
        result
    }

    pub(crate) fn commit(&mut self) {
        let shared = Rc::clone(&self.store);
        let store = &mut *shared.borrow_mut();
        self.publish_changed(store);
        store.tell_others(self.id);
        let Commit { save, problem, .. } = store.saving.commit_rune(
            &self.preferences,
            RunePreferences {
                transparency: self.transparency,
            },
        );
        match problem {
            Some(SaveProblem::UnreadableAtStartup { path }) => {
                if let Some(error) = store.saving.load_failure() {
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
        glib::MainContext::default().spawn_local(async move {
            let finished = save.finished().await;
            if let Some(later) = later {
                later.then(move |app: &mut App, _| crate::shell::saved(app).saved(finished));
            }
        });
    }

    pub(crate) fn change(&mut self, change: impl FnOnce(&mut Preferences)) {
        self.follow();
        let was_restore_tabs = self.preferences.restore_tabs;
        change(&mut self.preferences);
        if was_restore_tabs && !self.preferences.restore_tabs {
            strata_desktop::tabs_session::remove();
        }
        self.commit();
    }

    fn saved(&mut self, finished: FinishedSave) -> bool {
        let outcome = self.store.borrow_mut().saving.saved(finished);
        match outcome {
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

#[cfg(test)]
mod tests;
