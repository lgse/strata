// SPDX-License-Identifier: MIT

//! The bookkeeping of saving `settings.toml`: what changed since a save last succeeded, what
//! the file was last asked to hold, and whether saving is off for this session.
//!
//! It owns no values. An interface holds its [`Preferences`] however it holds state, changes
//! them, and then calls [`Saving::commit`]. A commit finds a change by comparing with the
//! values of the commit before, queues a save of the changed keys on the settings writer,
//! and hands the pending save back. Nothing here schedules or calls back: the interface
//! awaits the save where it awaits things and tells [`Saving::saved`] how it went.

use std::{collections::BTreeSet, future::Future, io, path::PathBuf, pin::Pin};

use super::{
    InterfaceRenderer, Preferences, RETIRED_SETTINGS, SettingKey, TenxerFx, merged_settings,
    read_loaded_preferences, record_changes, settings_path, tenxer_fx::read_saved_tenxer_fx,
};
use crate::storage::{
    file_stamp,
    writer::{self, Policy},
};

/// Reads the file, salvaged entry by entry and brought within range, and starts the
/// bookkeeping from it: what loading repaired is saved with the first change.
///
/// It applies nothing process-wide. The caller makes [`Saving::startup_locale`] the
/// process's language, and applies whatever else its interface takes from the values.
///
/// Call it once in a process. See [`Saving`].
pub fn load() -> (Preferences, Saving) {
    let (preferences, _, saving) = load_internal(false);
    (preferences, saving)
}

/// Like [`load`], also reading Rune's `tenxer_fx` setting from `settings.toml` so changes to
/// it merge and save through the same writer without altering [`Preferences`] for GTK.
pub fn load_with_tenxer_fx() -> (Preferences, TenxerFx, Saving) {
    load_internal(true)
}

fn load_internal(include_tenxer_fx: bool) -> (Preferences, TenxerFx, Saving) {
    writer::note(&settings_path(), file_stamp(&settings_path()));
    let (mut preferences, load_failure, salvaged) = read_loaded_preferences();
    let (tenxer_fx, repaired_fx) = if include_tenxer_fx {
        read_saved_tenxer_fx()
    } else {
        (TenxerFx::default(), false)
    };
    let parsed = preferences.clone();
    preferences.normalize_loaded();
    let mut saved_base = toml::Table::try_from(&preferences).unwrap_or_default();
    let mut repaired: BTreeSet<SettingKey> = salvaged.into_iter().map(|key| (key, None)).collect();
    record_changes(
        &mut repaired,
        &toml::Table::try_from(&parsed).unwrap_or_default(),
        &saved_base,
    );
    if include_tenxer_fx {
        saved_base.insert(TenxerFx::KEY.to_owned(), tenxer_fx.to_toml_value());
        if repaired_fx {
            repaired.insert((TenxerFx::KEY.to_owned(), None));
        }
    }
    let saving = Saving {
        startup_interface_renderer: preferences.interface_renderer,
        startup_locale: preferences.language.locale(),
        load_failure,
        baseline: preferences.clone(),
        baseline_tenxer_fx: include_tenxer_fx.then_some(tenxer_fx),
        revision: 0,
        unsaved: false,
        generation: 0,
        last_queued: saved_base,
        dirty_keys: repaired,
    };
    (preferences, tenxer_fx, saving)
}

/// The bookkeeping of one process's saves of `settings.toml`.
///
/// **One for the file in a process.** The settings writer coalesces the saves queued for a
/// path and gives every waiting save the result of the newest contents, so a second `Saving`
/// would be told its save succeeded and forget keys that were never written.
pub struct Saving {
    startup_interface_renderer: InterfaceRenderer,
    startup_locale: &'static str,
    /// Why saving is off for this session: the file existed but could not be read.
    load_failure: Option<io::Error>,
    /// The values as of the last commit, to see whether the next one changes anything.
    baseline: Preferences,
    /// Tracked `tenxer_fx` value for Rune processes; `None` in GTK processes.
    baseline_tenxer_fx: Option<TenxerFx>,
    revision: u64,
    /// A save is owed: one is on its way, or the last one failed.
    unsaved: bool,
    generation: u64,
    /// The settings as of the last load or queued save, to see what a save changes.
    last_queued: toml::Table,
    /// Changed or repaired since a save last succeeded; a save writes only these.
    dirty_keys: BTreeSet<SettingKey>,
}

/// What a commit did. Dropping `save` unawaited loses nothing on disk, but the keys it wrote
/// stay marked as changed and are written again with every later save.
#[must_use = "a queued save is awaited and its result given to `Saving::saved`"]
pub struct Commit {
    /// Whether a value differs from the commit before.
    pub changed: bool,
    pub save: Option<PendingSave>,
    pub problem: Option<SaveProblem>,
}

/// A save on its way to the file.
pub struct PendingSave {
    generation: u64,
    path: PathBuf,
    done: Pin<Box<dyn Future<Output = io::Result<()>>>>,
}

impl PendingSave {
    /// Waits for the writer. A writer that went away counts as a cancelled write.
    pub async fn finished(self) -> FinishedSave {
        FinishedSave {
            generation: self.generation,
            path: self.path,
            result: self.done.await,
        }
    }
}

/// The result of one queued save, for [`Saving::saved`].
pub struct FinishedSave {
    generation: u64,
    path: PathBuf,
    result: io::Result<()>,
}

/// Why a commit queued no save although one is owed.
pub enum SaveProblem {
    /// The file could not be read at start-up, so saving stays off to keep it. Reported for a
    /// commit that changed a value; [`Saving::load_failure`] has the error.
    UnreadableAtStartup { path: PathBuf },
    /// The values could not be put in the file's form. Each later commit tries again.
    Snapshot { path: PathBuf, error: io::Error },
}

pub enum SaveOutcome {
    Saved,
    /// Writing failed; each later commit tries again.
    Failed {
        path: PathBuf,
        error: io::Error,
    },
    /// A later save was queued meanwhile, and its result is the one that counts.
    Superseded,
}

impl Saving {
    /// Records what `preferences` changed since the commit before and queues a save of the
    /// keys changed since a save last succeeded. A commit that changes nothing still queues
    /// one while a save is owed, because a retry with unchanged values can fail too.
    pub fn commit(&mut self, preferences: &Preferences) -> Commit {
        self.commit_internal(preferences, None)
    }

    /// Like [`Self::commit`], also persisting Rune's `tenxer_fx` preference into `settings.toml`.
    pub fn commit_with_tenxer_fx(
        &mut self,
        preferences: &Preferences,
        tenxer_fx: TenxerFx,
    ) -> Commit {
        self.commit_internal(preferences, Some(tenxer_fx))
    }

    fn commit_internal(
        &mut self,
        preferences: &Preferences,
        tenxer_fx: Option<TenxerFx>,
    ) -> Commit {
        let mut changed = self.record(preferences);
        if let Some(fx) = tenxer_fx
            && self.baseline_tenxer_fx != Some(fx)
        {
            self.baseline_tenxer_fx = Some(fx);
            if !changed {
                self.revision = self.revision.wrapping_add(1);
            }
            changed = true;
        }
        if !changed && !self.unsaved {
            return Commit {
                changed,
                save: None,
                problem: None,
            };
        }
        let path = settings_path();
        if self.load_failure.is_some() {
            return Commit {
                changed,
                save: None,
                problem: changed.then_some(SaveProblem::UnreadableAtStartup { path }),
            };
        }
        self.unsaved = true;
        match toml::Table::try_from(preferences).map_err(io::Error::other) {
            Ok(mut snapshot) => {
                if let Some(fx) = tenxer_fx {
                    snapshot.insert(TenxerFx::KEY.to_owned(), fx.to_toml_value());
                }
                Commit {
                    changed,
                    save: Some(self.queue(path, snapshot, preferences)),
                    problem: None,
                }
            }
            Err(error) => Commit {
                changed,
                save: None,
                problem: Some(SaveProblem::Snapshot { path, error }),
            },
        }
    }

    fn queue(
        &mut self,
        path: PathBuf,
        snapshot: toml::Table,
        preferences: &Preferences,
    ) -> PendingSave {
        self.generation += 1;
        record_changes(&mut self.dirty_keys, &self.last_queued, &snapshot);
        self.last_queued = snapshot.clone();
        // Never serialized, so a migration cannot show up as a change.
        if preferences.legacy_omarchy_variant.is_none() {
            self.dirty_keys.insert(("omarchy_variant".to_owned(), None));
        }
        self.dirty_keys
            .extend(RETIRED_SETTINGS.map(|key| (key.to_owned(), None)));
        let keys = self.dirty_keys.clone();
        let target = path.clone();
        let saved = writer::save_locked(path.clone(), Policy::Config, move || {
            merged_settings(&target, &snapshot, &keys)
        });
        PendingSave {
            generation: self.generation,
            path,
            done: Box::pin(async move {
                saved
                    .await
                    .unwrap_or_else(|_| Err(io::Error::from(rustix::io::Errno::CANCELED)))
            }),
        }
    }

    /// Takes the result of a queued save. Only the latest save's result counts.
    pub fn saved(&mut self, finished: FinishedSave) -> SaveOutcome {
        if finished.generation != self.generation {
            return SaveOutcome::Superseded;
        }
        match finished.result {
            Ok(()) => {
                self.unsaved = false;
                self.dirty_keys.clear();
                SaveOutcome::Saved
            }
            Err(error) => SaveOutcome::Failed {
                path: finished.path,
                error,
            },
        }
    }

    /// Takes `keys` of `preferences` as what the file holds, for values another Strata
    /// process saved and this one adopted: they are not saved again. Returns whether a value
    /// differs from the commit before.
    ///
    /// The caller adopts a key only while [`Self::setting_unsaved`] is false for it. A save
    /// on its way has already taken this process's value for an unsaved key, and would write
    /// it over the other process's.
    pub fn record_adopted(&mut self, preferences: &Preferences, keys: &[&str]) -> bool {
        if let Ok(current) = toml::Table::try_from(preferences) {
            for key in keys {
                match current.get(*key) {
                    Some(value) => self.last_queued.insert((*key).to_owned(), value.clone()),
                    None => self.last_queued.remove(*key),
                };
                self.dirty_keys.remove(&((*key).to_owned(), None));
            }
        }
        self.record(preferences)
    }

    /// Makes `preferences` the values later commits compare with, without counting a change,
    /// so repairs made at start-up are not the next change. What they changed is saved with
    /// the next save.
    ///
    /// For start-up only, before the first commit. With a save on its way, that save's
    /// success would clear the keys this marks, and the repair would not be written until
    /// the value changed again.
    pub fn rebase(&mut self, preferences: &Preferences) {
        if let Ok(mut normalized) = toml::Table::try_from(preferences) {
            if let Some(fx) = self.baseline_tenxer_fx {
                normalized.insert(TenxerFx::KEY.to_owned(), fx.to_toml_value());
            }
            record_changes(&mut self.dirty_keys, &self.last_queued, &normalized);
            self.last_queued = normalized;
        }
        self.baseline = preferences.clone();
    }

    fn record(&mut self, preferences: &Preferences) -> bool {
        if self.baseline == *preferences {
            return false;
        }
        self.baseline = preferences.clone();
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Counts the commits and adoptions that changed a value: an interface that keeps the
    /// number it last saw knows when to read the values again.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether a save not yet written, or one that failed, would replace `key` on disk.
    pub fn setting_unsaved(&self, key: &str) -> bool {
        self.dirty_keys.iter().any(|(dirty, _)| dirty == key)
    }

    /// Whether a save is owed: one is on its way, or the last one failed.
    pub fn unsaved(&self) -> bool {
        self.unsaved
    }

    /// Why saving is off for this session: the file existed but could not be read.
    pub fn load_failure(&self) -> Option<&io::Error> {
        self.load_failure.as_ref()
    }

    pub fn startup_interface_renderer(&self) -> InterfaceRenderer {
        self.startup_interface_renderer
    }

    pub fn startup_locale(&self) -> &'static str {
        self.startup_locale
    }
}

#[cfg(test)]
mod tests;
