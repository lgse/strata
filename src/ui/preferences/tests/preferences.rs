// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use sourceview5::prelude::BufferExt as _;

use super::super::*;
use crate::{
    model::{
        EntryKind, FileEntry, FolderSort, Location, MetadataValue, SortDirection, SortKey,
        ViewPreferences,
    },
    services::SearchLimits,
    test_support::gtk_test,
    ui::{
        browser_modes::{BrowserDensity, BrowserMode, ClickCount, DensityScale, FrameDensity},
        keys::{Binding, Map, bindings},
        preferences::fixtures::{
            non_default_preferences, seed_omarchy_colors_for_test, seed_omarchy_for_test,
            seed_saved_preferences_for_test,
        },
        theme::ThemeManager,
    },
};

mod list_columns;
mod loading;
mod recovery;
mod relocations;
mod saved_fixture;
mod saving;
mod setters;
mod themes;

fn assert_recovered_preferences_survive_save(
    corrupt: impl FnOnce(&mut toml::Table),
    mut expected: Preferences,
) {
    seed_saved_preferences_for_test();
    let mut saved = toml::Table::try_from(non_default_preferences()).expect("saved preferences");
    corrupt(&mut saved);
    let malformed_keys: Vec<String> = saved.keys().cloned().collect();
    let malformed = toml::to_string(&saved).expect("syntactically valid TOML");
    fs::write(settings_path(), &malformed).expect("persist malformed preferences");

    let manager = PreferenceManager::shared();
    assert_eq!(*manager.preferences.borrow(), expected);
    crate::storage::writer::settle();
    assert_eq!(
        fs::read_to_string(settings_path()).expect("unchanged settings file"),
        malformed
    );
    expected.folder_peeking = false;
    manager.set_folder_peeking(false);
    crate::storage::writer::settle();

    let saved = fs::read_to_string(settings_path()).expect("saved file");
    let persisted: Preferences = toml::from_str(&saved).expect("save repairs invalid preferences");
    assert_eq!(persisted, expected);
    let saved_keys: BTreeSet<String> = toml::from_str::<toml::Table>(&saved)
        .expect("saved TOML")
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let every_key = toml::Table::try_from(non_default_preferences()).expect("every preference");
    let kept_keys: BTreeSet<String> = toml::Table::try_from(&expected)
        .expect("known preferences")
        .into_iter()
        .map(|(key, _)| key)
        .chain(malformed_keys.into_iter().filter(|key| {
            !every_key.contains_key(key) && !RETIRED_SETTINGS.contains(&key.as_str())
        }))
        .collect();
    assert_eq!(
        saved_keys, kept_keys,
        "a save keeps the valid and unknown preferences"
    );
    crate::storage::writer::settle();
    assert_eq!(read_preferences().expect("saved preferences"), expected);
}

fn wait_for(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "waiting for {what}");
        glib::MainContext::default().iteration(false);
    }
}
