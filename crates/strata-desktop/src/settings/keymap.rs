// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
