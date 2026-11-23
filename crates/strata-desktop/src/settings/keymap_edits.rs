// SPDX-License-Identifier: MIT

use std::{collections::BTreeMap, path::Path};

use super::KeymapOverrides;
use crate::keys::{
    Binding, Command, Keymap, Map,
    conflicts::{self, Policy, Severity},
};

fn entries_mut(overrides: &mut KeymapOverrides, map: Map) -> &mut BTreeMap<String, toml::Value> {
    match map {
        Map::Default => &mut overrides.default,
        Map::Tenxer => &mut overrides.tenxer,
    }
}

impl KeymapOverrides {
    pub fn set_bindings(&mut self, map: Map, command: Command, bindings: Vec<Binding>) {
        let mut unique = Vec::with_capacity(bindings.len());
        for binding in bindings.into_iter().map(Binding::normalized) {
            if !unique.contains(&binding) {
                unique.push(binding);
            }
        }
        let entries = entries_mut(self, map);
        if unique == command.defaults(map) {
            entries.remove(command.id());
        } else {
            let accelerators = unique
                .iter()
                .map(|binding| toml::Value::String(binding.accelerator().into()))
                .collect();
            entries.insert(command.id().to_owned(), toml::Value::Array(accelerators));
        }
    }

    /// Removes `map`'s override for `command`, restoring its shipped bindings.
    pub fn reset_binding(&mut self, map: Map, command: Command) {
        entries_mut(self, map).remove(command.id());
    }

    pub fn reset_map(&mut self, map: Map) {
        entries_mut(self, map).retain(|id, _| Command::parse(id).is_none());
    }
}

pub fn report_keymap_conflicts(keymap: &Keymap, file: &Path, policy: Policy) {
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

#[cfg(all(test, feature = "xkb-keys"))]
mod tests;
