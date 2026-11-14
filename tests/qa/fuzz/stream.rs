// SPDX-License-Identifier: MIT

//! Properties of one stream, which the runner checks itself, on every target: here, that an
//! Undo the application followed at once with a Redo puts the fixture tree back as it was
//! before the Undo (`docs/testing.md`, "Invariants"). A failure is written into the Redo's
//! observation as an invariant, so it is a finding as the application's own are.

use serde_json::Value;

use super::{
    observe::{Observation, Outcome},
    project::{FS, INVARIANTS, differences},
};

/// The keys the alphabets undo and redo with, as the default key map binds them.
const UNDO: &[&str] = &["ctrl+z"];
const REDO: &[&str] = &["ctrl+shift+z", "ctrl+y"];

/// Checks the observation just taken, `now`, against the two before it, and adds what it
/// breaks to its invariants.
pub(crate) fn check(before: &[Observation], now: &mut Observation) {
    let [.., earlier, undone] = before else {
        return;
    };
    let took = |observation: &Observation, keys: &[&str]| {
        observation.outcome() == Outcome::Ok.name() && is_key(&observation.action, keys)
    };
    if !(took(undone, UNDO) && took(now, REDO)) {
        return;
    }
    // An Undo that changed nothing in the fixture undid something outside it, or nothing.
    if undone.value[FS] == earlier.value[FS] {
        return;
    }
    let in_tree = |field: &String| field.starts_with(FS) && field[FS.len()..].starts_with('.');
    let Some(path) = differences(&earlier.value, &now.value)
        .into_iter()
        .map(|difference| difference.field)
        .find(in_tree)
    else {
        return;
    };
    let line = format!(
        "undo-redo: the tree after {} is not the tree before {}: {path} differs",
        key(&now.action),
        key(&undone.action)
    );
    if let Some(invariants) = now.value[INVARIANTS].as_array_mut() {
        invariants.push(Value::String(line));
    }
}

fn key(action: &Value) -> &str {
    action["key"].as_str().unwrap_or_default()
}

fn is_key(action: &Value, keys: &[&str]) -> bool {
    keys.contains(&key(action))
}
