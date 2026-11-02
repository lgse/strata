// SPDX-License-Identifier: MIT

//! Reading `settings.toml`, salvaging what a malformed file still holds, and saving only what this process changed.

use super::*;

pub(super) fn read_state_file(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub(super) fn read_preferences() -> io::Result<Preferences> {
    read_preferences_checked().map(|(preferences, _)| preferences)
}

/// Also returns the keys salvage had to repair.
fn read_preferences_checked() -> io::Result<(Preferences, BTreeSet<String>)> {
    let Some(contents) = read_state_file(&settings_path())? else {
        return Ok((Preferences::default(), BTreeSet::new()));
    };
    parse_preferences_checked(&contents)
}

pub(super) fn parse_preferences(contents: &str) -> io::Result<Preferences> {
    parse_preferences_checked(contents).map(|(preferences, _)| preferences)
}

fn parse_preferences_checked(contents: &str) -> io::Result<(Preferences, BTreeSet<String>)> {
    let table: toml::Table = toml::from_str(contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    match table.clone().try_into() {
        Ok(preferences) => Ok((preferences, BTreeSet::new())),
        Err(error) => {
            tracing::warn!(%error, "settings file has invalid entries; keeping the valid ones");
            Ok(salvage_preferences(table))
        }
    }
}

const SETTINGS_LOCK_WAIT: Duration = Duration::from_millis(500);

/// Dropped by every save.
pub(super) const RETIRED_SETTINGS: [&str; 1] = ["auto_refresh_interval"];

/// A changed key, or one entry of a key in [`ENTRY_MERGED_SETTINGS`].
pub(super) type SettingKey = (String, Option<String>);

/// Maps whose entries are merged one by one, so entries another process added survive.
const ENTRY_MERGED_SETTINGS: [&str; 5] = [
    "omarchy_variants",
    "folder_colors",
    "custom_icons",
    "send_to_recent_destinations",
    "device_labels",
];

pub(super) fn record_changes(
    changed: &mut BTreeSet<SettingKey>,
    before: &toml::Table,
    after: &toml::Table,
) {
    let empty = toml::Table::new();
    for key in after.keys().chain(before.keys()) {
        let (old, new) = (before.get(key), after.get(key));
        if old != new {
            record_change(changed, key, entries(old, &empty), entries(new, &empty));
        }
    }
}

/// A value's entries, with a missing value as an empty table, or `None` for a non-table.
fn entries<'a>(value: Option<&'a toml::Value>, empty: &'a toml::Table) -> Option<&'a toml::Table> {
    value.map_or(Some(empty), toml::Value::as_table)
}

fn record_change(
    changed: &mut BTreeSet<SettingKey>,
    key: &str,
    old: Option<&toml::Table>,
    new: Option<&toml::Table>,
) {
    match (old, new) {
        (Some(old), Some(new)) if ENTRY_MERGED_SETTINGS.contains(&key) => {
            record_entry_changes(changed, key, old, new);
        }
        _ => {
            changed.insert((key.to_owned(), None));
        }
    }
}
/// Another Strata process, such as the portal chooser, saves the same file, so a save
/// replaces only what this process changed and keeps the rest as it is on disk,
/// including keys this version does not know. A file that cannot be read or parsed
/// is left alone.
fn merged_settings(
    path: &Path,
    snapshot: &toml::Table,
    changed: &BTreeSet<SettingKey>,
) -> io::Result<(Vec<u8>, Option<fs::File>)> {
    let lock = folder_view_settings::lock_saves(path, SETTINGS_LOCK_WAIT)
        .inspect_err(|error| {
            tracing::debug!(%error, path = %path.display(), "saving settings unlocked");
        })
        .ok();
    // Anything but a file is left for the write to report.
    let on_disk = if path.is_file() {
        read_state_file(path)?
    } else {
        None
    };
    let merged = match on_disk {
        None => snapshot.clone(),
        Some(contents) => {
            let mut table: toml::Table = toml::from_str(&contents)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            for (key, entry) in changed {
                merge_setting(&mut table, snapshot, key, entry.as_ref());
            }
            table
        }
    };
    let contents = toml::to_string_pretty(&merged).map_err(io::Error::other)?;
    Ok((contents.into_bytes(), lock))
}

fn record_entry_changes(
    changed: &mut BTreeSet<SettingKey>,
    key: &str,
    old: &toml::Table,
    new: &toml::Table,
) {
    for entry in new.keys().chain(old.keys()) {
        if old.get(entry) != new.get(entry) {
            changed.insert((key.to_owned(), Some(entry.clone())));
        }
    }
}

/// Copies one changed key, or one entry of it, from `snapshot` into `table`.
fn merge_setting(
    table: &mut toml::Table,
    snapshot: &toml::Table,
    key: &str,
    entry: Option<&String>,
) {
    let Some(entry) = entry else {
        match snapshot.get(key) {
            Some(value) => table.insert(key.to_owned(), value.clone()),
            None => table.remove(key),
        };
        return;
    };
    let value = snapshot
        .get(key)
        .and_then(toml::Value::as_table)
        .and_then(|entries| entries.get(entry));
    let slot = table
        .entry(key.to_owned())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !slot.is_table() {
        *slot = toml::Value::Table(toml::Table::new());
    }
    let Some(entries) = slot.as_table_mut() else {
        return;
    };
    match value {
        Some(value) => entries.insert(entry.clone(), value.clone()),
        None => entries.remove(entry),
    };
    if entries.is_empty() {
        table.remove(key);
    }
}
/// Rebuilds preferences from every entry that deserializes on its own, so one
/// malformed value does not reset the rest (and, on the next save, overwrite
/// them with defaults). A malformed list or table keeps its other entries.
/// Also returns the keys whose saved value was dropped or filled in.
pub(super) fn salvage_preferences(saved: toml::Table) -> (Preferences, BTreeSet<String>) {
    let Ok(mut merged) = toml::Table::try_from(Preferences::default()) else {
        return (
            Preferences::default(),
            saved.into_iter().map(|(key, _)| key).collect(),
        );
    };
    for (key, value) in saved.clone() {
        let kept = {
            let accepts = |candidate: &toml::Value| {
                let mut trial = merged.clone();
                trial.insert(key.clone(), candidate.clone());
                trial.try_into::<Preferences>().is_ok()
            };
            salvage_value(value, &accepts)
        };
        if let Some(kept) = kept {
            merged.insert(key, kept);
        }
    }
    let repaired = merged
        .keys()
        .chain(saved.keys())
        .filter(|key| merged.get(*key) != saved.get(*key))
        .cloned()
        .collect();
    (merged.try_into().unwrap_or_default(), repaired)
}

/// Returns `value` if `accepts` takes it, otherwise the entries that still pass
/// after dropping the bad ones, level by level. Entries are tried alone, so
/// salvage stays linear in a collection's size, and then checked together. A
/// value of the wrong shape, or a record missing a required field, yields `None`.
fn salvage_value(
    value: toml::Value,
    accepts: &dyn Fn(&toml::Value) -> bool,
) -> Option<toml::Value> {
    if accepts(&value) {
        return Some(value);
    }
    let kept = match value {
        toml::Value::Table(table) => toml::Value::Table(salvage_table(table, accepts)),
        toml::Value::Array(items) => toml::Value::Array(salvage_array(items, accepts)),
        _ => return None,
    };
    accepts(&kept).then_some(kept)
}

fn salvage_table(table: toml::Table, accepts: &dyn Fn(&toml::Value) -> bool) -> toml::Table {
    let mut kept = toml::Table::new();
    for (key, child) in table {
        let alone = |candidate: &toml::Value| {
            let trial = toml::Table::from_iter([(key.clone(), candidate.clone())]);
            accepts(&toml::Value::Table(trial))
        };
        if let Some(child) = salvage_value(child, &alone) {
            kept.insert(key, child);
        }
    }
    kept
}

fn salvage_array(
    items: Vec<toml::Value>,
    accepts: &dyn Fn(&toml::Value) -> bool,
) -> Vec<toml::Value> {
    let mut kept = Vec::new();
    for item in items {
        let alone = |candidate: &toml::Value| accepts(&toml::Value::Array(vec![candidate.clone()]));
        if let Some(item) = salvage_value(item, &alone) {
            kept.push(item);
        }
    }
    kept
}
pub(super) fn settings_path() -> PathBuf {
    config_directory().join("settings.toml")
}

/// The saved settings, or temporary defaults with the read error when the file
/// exists but cannot be read; also returns the keys salvage repaired.
pub(super) fn read_loaded_preferences() -> (Preferences, Option<io::Error>, BTreeSet<String>) {
    match read_preferences_checked() {
        Ok((preferences, salvaged)) => (preferences, None, salvaged),
        Err(error) => {
            tracing::warn!(%error, path = %settings_path().display(),
                "unable to load settings; using temporary defaults without saving; fix the file and restart Strata");
            (Preferences::default(), Some(error), BTreeSet::new())
        }
    }
}

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
