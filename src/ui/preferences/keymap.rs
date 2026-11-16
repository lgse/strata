// SPDX-License-Identifier: MIT

//! The saved keymap overrides (`strata_desktop::settings::KeymapOverrides`)
//! resolved against the key catalog, and the edits Settings makes to them.
//! See the Keymap section of `docs/preferences.md`. Resolving and editing them
//! are shared with the Rune interface (`strata_desktop::settings`).

use std::rc::Rc;

use strata_desktop::settings::{report_keymap_conflicts, resolve_keymap};

use super::{KeymapOverrides, PreferenceManager, settings_path};
use crate::ui::keys::{Binding, Command, Keymap, Map, conflicts::Policy};

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
        report_keymap_conflicts(
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
        self.edit_keymap(|overrides| overrides.set_bindings(map, command, bindings));
    }

    pub(crate) fn reset_binding(&self, map: Map, command: Command) {
        self.edit_keymap(|overrides| overrides.reset_binding(map, command));
    }

    /// Restores every command of `map`; entries for ids this version does not
    /// know stay saved.
    pub(crate) fn reset_map(&self, map: Map) {
        self.edit_keymap(|overrides| overrides.reset_map(map));
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
