// SPDX-License-Identifier: MIT

//! Saving `settings.toml`: only what this process changed, off the interface thread. The
//! bookkeeping is `strata_desktop::settings::Saving`; this publishes the change to the
//! bindings, awaits the save on GLib's context and tells the notices.

use super::*;

impl PreferenceManager {
    pub(super) fn save_preferences(&self) {
        let changed = self.changes.record(&self.preferences.borrow());
        let commit = self.saving.borrow_mut().commit(&self.preferences.borrow());
        debug_assert_eq!(
            changed, commit.changed,
            "the bindings and the save bookkeeping compare with the same values"
        );
        if let Some(save) = commit.save {
            self.await_settings_save(save);
        }
        if changed {
            self.changes.notify(self);
        }
        // Reported after bindings apply, and whether or not anything changed,
        // because a retry with an unchanged value can fail too.
        // Named in full: `save_notice` has a `SaveProblem` of its own.
        match commit.problem {
            Some(strata_desktop::settings::SaveProblem::UnreadableAtStartup { path }) => {
                if let Some(error) = self.saving.borrow().load_failure() {
                    self.save_notices.unreadable_at_startup(&path, error);
                }
            }
            Some(strata_desktop::settings::SaveProblem::Snapshot { path, error }) => {
                self.save_notices.write_failed(&path, &error);
            }
            None => {}
        }
    }

    fn await_settings_save(&self, save: PendingSave) {
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let finished = save.finished().await;
            if let Some(manager) = this.upgrade() {
                manager.settings_saved(finished);
            }
        });
    }

    fn settings_saved(&self, finished: FinishedSave) {
        let outcome = self.saving.borrow_mut().saved(finished);
        match outcome {
            SaveOutcome::Saved => self.save_notices.saved(),
            SaveOutcome::Failed { path, error } => self.save_notices.write_failed(&path, &error),
            SaveOutcome::Superseded => {}
        }
    }

    /// Whether a save not yet written, or one that failed, would replace `key` on disk.
    pub(super) fn setting_unsaved(&self, key: &str) -> bool {
        self.saving.borrow().setting_unsaved(key)
    }
}
