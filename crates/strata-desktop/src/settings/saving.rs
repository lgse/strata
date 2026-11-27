// SPDX-License-Identifier: MIT

use std::{collections::BTreeSet, future::Future, io, path::PathBuf, pin::Pin};

use super::{
    InterfaceRenderer, Preferences, RETIRED_SETTINGS, SettingKey, TransparencyPreferences,
    merged_settings, read_loaded_preferences, record_changes, settings_path,
    transparency::read_saved_transparency,
};
use crate::storage::{
    file_stamp,
    writer::{self, Policy},
};

/// Rune-only visual preferences persisted alongside [`Preferences`] in `settings.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunePreferences {
    pub transparency: TransparencyPreferences,
}

#[derive(Clone, Copy, Default)]
struct RuneTracking {
    transparency: bool,
}

#[derive(Clone, Copy, Default)]
struct RuneCommit {
    transparency: Option<TransparencyPreferences>,
}

pub fn load() -> (Preferences, Saving) {
    let (preferences, _, saving) = load_internal(RuneTracking::default());
    (preferences, saving)
}

pub fn load_rune() -> (Preferences, RunePreferences, Saving) {
    load_internal(RuneTracking { transparency: true })
}

fn load_internal(track: RuneTracking) -> (Preferences, RunePreferences, Saving) {
    writer::note(&settings_path(), file_stamp(&settings_path()));
    let (mut preferences, load_failure, salvaged) = read_loaded_preferences();
    let (transparency, repaired_transparency) = if track.transparency {
        read_saved_transparency()
    } else {
        (TransparencyPreferences::default(), Vec::new())
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
    if track.transparency {
        transparency.insert_into_table(&mut saved_base);
        for key in repaired_transparency {
            repaired.insert((key.to_owned(), None));
        }
    }
    let saving = Saving {
        startup_interface_renderer: preferences.interface_renderer,
        startup_locale: preferences.language.locale(),
        load_failure,
        baseline: preferences.clone(),
        baseline_transparency: track.transparency.then_some(transparency),
        revision: 0,
        unsaved: false,
        generation: 0,
        last_queued: saved_base,
        dirty_keys: repaired,
    };
    (preferences, RunePreferences { transparency }, saving)
}

pub struct Saving {
    startup_interface_renderer: InterfaceRenderer,
    startup_locale: &'static str,
    /// Why saving is off for this session: the file existed but could not be read.
    load_failure: Option<io::Error>,
    /// The values as of the last commit, to see whether the next one changes anything.
    baseline: Preferences,
    baseline_transparency: Option<TransparencyPreferences>,
    revision: u64,
    /// A save is owed: one is on its way, or the last one failed.
    unsaved: bool,
    generation: u64,
    /// The settings as of the last load or queued save, to see what a save changes.
    last_queued: toml::Table,
    /// Changed or repaired since a save last succeeded; a save writes only these.
    dirty_keys: BTreeSet<SettingKey>,
}

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
    UnreadableAtStartup {
        path: PathBuf,
    },
    /// The values could not be put in the file's form. Each later commit tries again.
    Snapshot {
        path: PathBuf,
        error: io::Error,
    },
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
    pub fn commit(&mut self, preferences: &Preferences) -> Commit {
        self.commit_internal(preferences, RuneCommit::default())
    }

    pub fn commit_rune(&mut self, preferences: &Preferences, rune: RunePreferences) -> Commit {
        self.commit_internal(
            preferences,
            RuneCommit {
                transparency: Some(rune.transparency.normalized()),
            },
        )
    }

    fn record_rune(&mut self, rune: RuneCommit, already_changed: bool) -> bool {
        let mut changed = already_changed;
        if let Some(transparency) = rune.transparency
            && self.baseline_transparency != Some(transparency)
        {
            self.baseline_transparency = Some(transparency);
            if !changed {
                self.revision = self.revision.wrapping_add(1);
            }
            changed = true;
        }
        changed
    }

    fn commit_internal(&mut self, preferences: &Preferences, rune: RuneCommit) -> Commit {
        let changed = self.record(preferences);
        let changed = self.record_rune(rune, changed);
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
                if let Some(transparency) = rune.transparency {
                    transparency.insert_into_table(&mut snapshot);
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

    pub fn rebase(&mut self, preferences: &Preferences) {
        if let Ok(mut normalized) = toml::Table::try_from(preferences) {
            if let Some(transparency) = self.baseline_transparency {
                transparency.insert_into_table(&mut normalized);
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
