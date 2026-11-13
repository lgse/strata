// SPDX-License-Identifier: MIT

//! The saved keymap overrides ([`KeymapOverrides`]) resolved against the key catalog, for
//! both interfaces: `resolve` and `report_capital_letters` of `src/ui/preferences/keymap.rs`,
//! moved as text. See the Keymap section of `docs/preferences.md`.
//!
//! The accelerator strings are read by the key library the interface installed, so on the
//! Rune side a saved keymap reads as it does in the GTK application only while `keys::Xkb`
//! reads them as GTK does.

use std::{collections::BTreeMap, path::Path};

use super::KeymapOverrides;
use crate::keys::{Keymap, Map, Rejected};

/// The shipped tables with every entry of `overrides` that applies. With `file`, the entries
/// that cannot apply are logged against it, and so is an uppercase letter written without
/// `<Shift>`.
pub fn resolve_keymap(overrides: &KeymapOverrides, file: Option<&Path>) -> Keymap {
    if let Some(file) = file {
        for (map, entries) in [
            (Map::Default, &overrides.default),
            (Map::Tenxer, &overrides.tenxer),
        ] {
            report_capital_letters(file, map, entries);
        }
    }
    Keymap::defaults()
        .overridden(
            Map::Default,
            entries(&overrides.default),
            report(file, Map::Default),
        )
        .overridden(
            Map::Tenxer,
            entries(&overrides.tenxer),
            report(file, Map::Tenxer),
        )
}

fn report(file: Option<&Path>, map: Map) -> impl FnMut(Rejected<'_>) + '_ {
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

/// They hold it through `keys::Xkb`, which is behind its feature.
#[cfg(all(test, feature = "xkb-keys"))]
mod tests;
