// SPDX-License-Identifier: MIT

//! Saved keymap overrides: only the entries a user changed, per map, kept as
//! written so a newer version's ids and keys survive an older version's save.
//! See the Keymap section of `docs/preferences.md`.

use std::{collections::BTreeMap, path::Path, rc::Rc};

use serde::{Deserialize, Serialize};

use super::{PreferenceManager, settings_path};
use crate::ui::keys::{
    Binding, Command, Keymap, Map, Rejected,
    conflicts::{self, Policy, Severity},
};

/// `[keymap.default]` and `[keymap.tenxer]`: command ids and their
/// accelerators, primary first. An empty list unbinds the command.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(in crate::ui) struct KeymapOverrides {
    #[serde(
        default,
        deserialize_with = "entries_or_empty",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub(super) default: BTreeMap<String, toml::Value>,
    #[serde(
        default,
        deserialize_with = "entries_or_empty",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub(super) tenxer: BTreeMap<String, toml::Value>,
}

/// A map that is not a table is dropped alone, so the other map's entries
/// survive the salvage of a malformed file.
fn entries_or_empty<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, toml::Value>, D::Error> {
    Ok(match toml::Value::deserialize(deserializer)? {
        toml::Value::Table(entries) => entries.into_iter().collect(),
        other => {
            tracing::warn!(%other, "keymap map is not a table of entries; dropping it");
            BTreeMap::new()
        }
    })
}

impl KeymapOverrides {
    pub(super) fn is_empty(&self) -> bool {
        self.default.is_empty() && self.tenxer.is_empty()
    }

    fn entries_mut(&mut self, map: Map) -> &mut BTreeMap<String, toml::Value> {
        match map {
            Map::Default => &mut self.default,
            Map::Tenxer => &mut self.tenxer,
        }
    }

    /// The shipped tables with every entry that applies. With `file`, the
    /// entries that cannot apply are logged against it.
    fn resolve(&self, file: Option<&Path>) -> Keymap {
        let report = |map: Map| {
            move |rejected: Rejected<'_>| {
                let Some(file) = file else {
                    return;
                };
                let file = file.display();
                match rejected {
                    Rejected::UnknownCommand(id) => tracing::warn!(
                        %file, ?map, id,
                        "keymap entry names no command; keeping it unused"
                    ),
                    Rejected::Table(id) => tracing::warn!(
                        %file, ?map, id,
                        "keymap entry is a table; quote ids that contain a dot, such as \"go.home\"; keeping it unused"
                    ),
                    Rejected::NotAList(id) => tracing::warn!(
                        %file, ?map, id,
                        "keymap entry is not a list of accelerators; keeping it unused"
                    ),
                    Rejected::Accelerator(id, accelerator) => tracing::warn!(
                        %file, ?map, id, %accelerator,
                        "keymap entry has an accelerator Strata cannot read; keeping it unused"
                    ),
                }
            }
        };
        if let Some(file) = file {
            for (map, entries) in [(Map::Default, &self.default), (Map::Tenxer, &self.tenxer)] {
                report_capital_letters(file, map, entries);
            }
        }
        Keymap::defaults()
            .overridden(Map::Default, entries(&self.default), report(Map::Default))
            .overridden(Map::Tenxer, entries(&self.tenxer), report(Map::Tenxer))
    }
}

fn entries(entries: &BTreeMap<String, toml::Value>) -> impl Iterator<Item = (&str, &toml::Value)> {
    entries.iter().map(|(id, value)| (id.as_str(), value))
}

/// GTK reads `G` as plain **g**, while labels show Shift+g as **G**, so an
/// uppercase letter without `<Shift>` likely means a key the user did not get.
fn report_capital_letters(file: &Path, map: Map, entries: &BTreeMap<String, toml::Value>) {
    let file = file.display();
    for (id, value) in entries {
        let accelerators = value.as_array().into_iter().flatten();
        for accelerator in accelerators.filter_map(toml::Value::as_str) {
            let key = accelerator.rsplit('>').next().unwrap_or(accelerator);
            let capital = key.len() == 1 && key.chars().all(|letter| letter.is_ascii_uppercase());
            if capital && !accelerator.to_ascii_lowercase().contains("<shift>") {
                tracing::warn!(
                    %file, ?map, id, accelerator,
                    "keymap accelerator names an uppercase letter without <Shift>, which means the plain letter; write <Shift> with it for the shifted key"
                );
            }
        }
    }
}

/// Logs what the checker finds about the keys the file's entries add. The
/// entries still apply.
fn report_findings(keymap: &Keymap, file: &Path, policy: Policy) {
    let file = file.display();
    for map in [Map::Default, Map::Tenxer] {
        for (command, finding) in conflicts::audit_added(keymap, map, policy) {
            let id = command.id();
            let accelerator = finding.binding.accelerator();
            let kind = finding.kind;
            match finding.severity {
                Severity::Error => tracing::warn!(
                    %file, ?map, id, %accelerator, ?kind,
                    "keymap entry has a key that never fires there or stops another key working"
                ),
                Severity::Warning => tracing::info!(
                    %file, ?map, id, %accelerator, ?kind,
                    "keymap entry has a key that works only in some places"
                ),
            }
        }
    }
}

impl PreferenceManager {
    /// Both maps' tables with the saved overrides applied. Built on first use,
    /// after GTK can parse accelerators, and logged once for the file's
    /// entries that cannot apply and the conflicts their keys cause.
    pub(crate) fn keymap(&self) -> Rc<Keymap> {
        if let Some(keymap) = self.keymap.borrow().as_ref() {
            return keymap.clone();
        }
        let file = settings_path();
        let keymap = Rc::new(self.preferences.borrow().keymap.resolve(Some(&file)));
        report_findings(
            &keymap,
            &file,
            Policy {
                type_to_search: self.type_to_search(),
            },
        );
        self.keymap.replace(Some(keymap.clone()));
        keymap
    }

    /// Changes whenever an override does, for bindings that render keys.
    pub(crate) fn keymap_revision(&self) -> u64 {
        self.keymap_revision.get()
    }

    /// What a shortcut label depends on: the active map, whether
    /// type-to-search types the keys a user added, and the keys.
    pub(crate) fn shown_keys(&self) -> (bool, bool, u64) {
        (
            self.tenxer_mode(),
            self.type_to_search_active(),
            self.keymap_revision(),
        )
    }

    /// Saves `bindings`, primary first, as `map`'s entry for `command`. The
    /// shipped bindings remove the override; an empty list unbinds.
    pub(crate) fn set_bindings(&self, map: Map, command: Command, bindings: Vec<Binding>) {
        let mut unique = Vec::with_capacity(bindings.len());
        for binding in bindings.into_iter().map(Binding::normalized) {
            if !unique.contains(&binding) {
                unique.push(binding);
            }
        }
        self.edit_keymap(|overrides| {
            let entries = overrides.entries_mut(map);
            if unique == command.defaults(map) {
                entries.remove(command.id());
            } else {
                let accelerators = unique
                    .iter()
                    .map(|binding| toml::Value::String(binding.accelerator().into()))
                    .collect();
                entries.insert(command.id().to_owned(), toml::Value::Array(accelerators));
            }
        });
    }

    pub(crate) fn reset_binding(&self, map: Map, command: Command) {
        self.edit_keymap(|overrides| {
            overrides.entries_mut(map).remove(command.id());
        });
    }

    /// Restores every command of `map`; entries for ids this version does not
    /// know stay saved.
    pub(crate) fn reset_map(&self, map: Map) {
        self.edit_keymap(|overrides| {
            overrides
                .entries_mut(map)
                .retain(|id, _| Command::parse(id).is_none());
        });
    }

    /// Rebuilds the tables before the change publishes, so bindings that
    /// render keys read the new ones.
    fn edit_keymap(&self, edit: impl FnOnce(&mut KeymapOverrides)) {
        // Reports the loaded file's entries before an edit replaces them.
        self.keymap();
        let rebuilt = {
            let mut preferences = self.preferences.borrow_mut();
            let previous = preferences.keymap.clone();
            edit(&mut preferences.keymap);
            (preferences.keymap != previous).then(|| preferences.keymap.resolve(None))
        };
        if let Some(keymap) = rebuilt {
            self.keymap.replace(Some(Rc::new(keymap)));
            self.keymap_revision
                .set(self.keymap_revision.get().wrapping_add(1));
        }
        self.save_preferences();
    }
}
