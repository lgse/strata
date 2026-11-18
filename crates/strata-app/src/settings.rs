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

use std::{cell::RefCell, io, path::Path, rc::Rc};

use strata_core::model::ViewPreferences;
use strata_desktop::settings::{
    ADOPTED_DEFAULT_KEYS, Commit, FinishedSave, Preferences, SaveOutcome, SaveProblem, Saving,
    ThemeAvailability,
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

/// The settings every window of the process shares: the values, the bookkeeping of saving
/// them, and the windows to tell when one of them changes them. One for the process, made at
/// start-up: two would lose each other's saves (`strata_desktop::settings::Saving`), as the
/// GTK application keeps one `PreferenceManager` for all its windows.
struct Store {
    preferences: Preferences,
    saving: Saving,
    /// Moves with every change any window makes.
    version: u64,
    /// The windows reading the store, each by its [`Saved::id`], to be told of a change.
    windows: Vec<(u64, WeakLater<App>)>,
    /// The id the next window's [`Saved`] takes.
    next: u64,
}

impl Store {
    /// Queues a [`Saved::follow`] in every window but `changed_by`, through the shell, and
    /// wakes each to apply it: a window without the keyboard gets no frame of its own to.
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

/// A window's way into the process's settings. It reads the values as of the last change it
/// heard of ([`Saved::follow`]), so a build borrows them as plain data; a change it makes
/// goes into the store at once, and the other windows hear of it through their queues.
pub(crate) struct Saved {
    store: Rc<RefCell<Store>>,
    /// This window's place among the store's windows.
    id: u64,
    /// The values as of `version`.
    preferences: Preferences,
    version: u64,
    /// The window's queue, where a finished save tells the bookkeeping its result. Without
    /// it a save is still written, and its keys are written again with the next.
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
    /// Reads the file, salvaged entry by entry and brought within range, and makes its
    /// language the process's.
    pub(crate) fn load() -> Self {
        let (preferences, saving) = strata_desktop::settings::load();
        rust_i18n::set_locale(saving.startup_locale());
        let store = Store {
            preferences: preferences.clone(),
            saving,
            version: 0,
            windows: Vec::new(),
            next: 1,
        };
        Self {
            store: Rc::new(RefCell::new(store)),
            id: 0,
            preferences,
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
            version: store.version,
            later: None,
            notices: NoticeStreaks::default(),
        }
    }

    /// Gives the settings the window's queue, once the window has one: a finished save is
    /// heard there, and so is another window's change.
    pub(crate) fn report_to(&mut self, later: Later<App>) {
        let mut store = self.store.borrow_mut();
        store.windows.retain(|(id, _)| *id != self.id);
        store.windows.push((self.id, later.downgrade()));
        self.later = Some(later);
    }

    pub(crate) fn preferences(&self) -> &Preferences {
        &self.preferences
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

    /// Takes the values another window changed since this one last read them. False when
    /// none did.
    pub(crate) fn follow(&mut self) -> bool {
        let store = self.store.borrow();
        if store.version == self.version {
            return false;
        }
        self.preferences = store.preferences.clone();
        self.version = store.version;
        true
    }

    /// Puts this window's values into the store, as the process's from now on.
    fn publish(&mut self, store: &mut Store) {
        store.preferences = self.preferences.clone();
        store.version += 1;
        self.version = store.version;
    }

    /// Puts this window's values into the store when they differ from the process's.
    fn publish_changed(&mut self, store: &mut Store) {
        if store.preferences != self.preferences {
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

    /// Adopts default sort and icon-size keys another Strata process wrote, provided none of
    /// those keys have unsaved edits in this process.
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
    /// What it changed is the process's at once; the other windows hear of it at the commit.
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

    /// Commits whatever changed in `Preferences` since the last save and queues the write off
    /// the interface thread. A file that could not be read at start-up is the user's to repair,
    /// and nothing is saved over it.
    pub(crate) fn commit(&mut self) {
        let shared = Rc::clone(&self.store);
        let store = &mut *shared.borrow_mut();
        self.publish_changed(store);
        store.tell_others(self.id);
        let Commit { save, problem, .. } = store.saving.commit(&self.preferences);
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
