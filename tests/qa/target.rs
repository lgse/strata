// SPDX-License-Identifier: MIT

//! The Rune QA target: its binary (`strata`), its artifact folder, and the protocol steps it
//! expects for layout switches and invariant checks.

use serde_json::{Value, json};

use crate::{BINARY, fixture::Layout};

/// The binary every scenario run starts.
pub(crate) fn binary() -> &'static str {
    BINARY
}

/// Where a run's artifacts go, under the checkout.
pub(crate) fn artifacts() -> &'static str {
    "target/qa-artifacts"
}

/// The step that tells the application which layout the runner switched to. The offscreen
/// host has no display, so it is sent the layout and types under it itself.
pub(crate) fn keymap_step(layout: &Layout) -> Value {
    json!({ "keymap": {
        "layouts": layout.layouts,
        "variants": layout.variants,
        "group": layout.group,
        "caps_lock": layout.caps_lock,
    } })
}

/// The step that asks the application for the invariants it holds itself to at rest.
#[expect(dead_code, reason = "kept for QA invariant checks")]
pub(crate) fn invariants_step() -> Value {
    json!({ "invariants": {} })
}
