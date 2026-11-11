// SPDX-License-Identifier: MIT

//! Saved keymap overrides: only the entries a user changed, per map, kept as
//! written so a newer version's ids and keys survive an older version's save.
//! See the Keymap section of `docs/preferences.md`. The interface resolves them
//! against its key catalog.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `[keymap.default]` and `[keymap.tenxer]`: command ids and their
/// accelerators, primary first. An empty list unbinds the command.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KeymapOverrides {
    #[serde(
        default,
        deserialize_with = "entries_or_empty",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub default: BTreeMap<String, toml::Value>,
    #[serde(
        default,
        deserialize_with = "entries_or_empty",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub tenxer: BTreeMap<String, toml::Value>,
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
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.tenxer.is_empty()
    }
}
