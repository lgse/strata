// SPDX-License-Identifier: MIT

//! The saved keymap overrides (`strata_desktop::settings::KeymapOverrides`)
//! resolved against the key catalog, and the edits Settings makes to them.
//! See the Keymap section of `docs/preferences.md`. Resolving them is shared
//! with the Rune interface (`strata_desktop::settings::resolve_keymap`).

use std::{collections::BTreeMap, path::Path, rc::Rc};

use strata_desktop::settings::resolve_keymap;

use super::{KeymapOverrides, PreferenceManager, settings_path};
use crate::ui::keys::{
    Binding, Command, Keymap, Map,
    conflicts::{self, Policy, Severity},
};

fn entries_mut(overrides: &mut KeymapOverrides, map: Map) -> &mut BTreeMap<String, toml::Value> {
    match map {
        Map::Default => &mut overrides.default,
        Map::Tenxer => &mut overrides.tenxer,
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
        let keymap = Rc::new(resolve_keymap(
            &self.preferences.borrow().keymap,
            Some(&file),
        ));
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
            let entries = entries_mut(overrides, map);
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
            entries_mut(overrides, map).remove(command.id());
        });
    }

    /// Restores every command of `map`; entries for ids this version does not
    /// know stay saved.
    pub(crate) fn reset_map(&self, map: Map) {
        self.edit_keymap(|overrides| {
            entries_mut(overrides, map).retain(|id, _| Command::parse(id).is_none());
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
            (preferences.keymap != previous).then(|| resolve_keymap(&preferences.keymap, None))
        };
        if let Some(keymap) = rebuilt {
            self.keymap.replace(Some(Rc::new(keymap)));
            self.keymap_revision
                .set(self.keymap_revision.get().wrapping_add(1));
        }
        self.save_preferences();
    }
}
