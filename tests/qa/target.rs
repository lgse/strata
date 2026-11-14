// SPDX-License-Identifier: MIT

//! Which interface the runner starts: the GTK application, or the one on Rune. The scenario
//! files, the fixtures and every runner step are the same for both.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{BINARY, fixture::Layout};

/// `gtk` (the default) or `rune`.
pub(crate) const TARGET_VARIABLE: &str = "STRATA_QA_TARGET";
/// The `strata-rune` binary built with its `qa` feature. Cargo gives a test only the binaries
/// of its own package, so the runner is told where this one is; `./scripts/qa-parity.sh` does.
pub(crate) const RUNE_BINARY_VARIABLE: &str = "STRATA_QA_RUNE_BINARY";

/// A scenario header's `targets` lists them by these names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Target {
    Gtk,
    Rune,
}

impl Target {
    pub(crate) fn named(name: Option<&str>) -> Result<Self, String> {
        match name {
            None | Some("" | "gtk") => Ok(Self::Gtk),
            Some("rune") => Ok(Self::Rune),
            Some(other) => Err(format!("{TARGET_VARIABLE} is gtk or rune, not {other:?}")),
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Gtk => "gtk",
            Self::Rune => "rune",
        }
    }

    pub(crate) fn from_environment() -> Result<Self, String> {
        match std::env::var(TARGET_VARIABLE) {
            Ok(name) => Self::named(Some(&name)),
            Err(std::env::VarError::NotPresent) => Self::named(None),
            Err(error) => Err(format!("{TARGET_VARIABLE}: {error}")),
        }
    }

    /// The binary a run starts. `rune` is what `STRATA_QA_RUNE_BINARY` holds.
    pub(crate) fn binary(self, rune: Option<&OsStr>) -> Result<PathBuf, String> {
        match self {
            Self::Gtk => Ok(PathBuf::from(BINARY)),
            Self::Rune => {
                let path = rune.filter(|path| !path.is_empty()).ok_or_else(|| {
                    format!(
                        "{TARGET_VARIABLE}=rune needs {RUNE_BINARY_VARIABLE}: the path of a \
                         strata-rune binary built with its qa feature. \
                         ./scripts/qa-parity.sh builds one and sets it."
                    )
                })?;
                if Path::new(path).is_file() {
                    Ok(PathBuf::from(path))
                } else {
                    Err(format!(
                        "{RUNE_BINARY_VARIABLE} names {}, which is not a file",
                        Path::new(path).display()
                    ))
                }
            }
        }
    }

    /// Where a run's artifacts go, under the checkout: each target keeps its own, so one
    /// command can run a scenario on both and leave both bundles.
    pub(crate) fn artifacts(self) -> &'static str {
        match self {
            Self::Gtk => "target/qa-artifacts",
            Self::Rune => "target/qa-artifacts-rune",
        }
    }

    /// The step that tells the application which layout the runner switched to. The GTK
    /// application types through the X display whose keymap the runner swapped; Rune's
    /// offscreen host has no display, so it is sent the layout and types under it itself.
    pub(crate) fn keymap_step(self, layout: &Layout) -> Option<Value> {
        match self {
            Self::Gtk => None,
            Self::Rune => Some(json!({ "keymap": {
                "layouts": layout.layouts,
                "variants": layout.variants,
                "group": layout.group,
                "caps_lock": layout.caps_lock,
            } })),
        }
    }

    /// The step that asks the application for the invariants it holds itself to at rest,
    /// which the fuzz driver sends after every step (`docs/testing.md`, "Invariants"). Only
    /// the Rune application checks any; the GTK one answers that it is not supported.
    pub(crate) fn invariants_step(self) -> Option<Value> {
        match self {
            Self::Gtk => None,
            Self::Rune => Some(json!({ "invariants": {} })),
        }
    }
}

/// The artifact folder of the target the environment names. A target it cannot name fails
/// the launch with the reason, and leaves what there is to keep where a GTK run would.
pub(crate) fn artifacts() -> &'static str {
    Target::from_environment()
        .unwrap_or(Target::Gtk)
        .artifacts()
}
