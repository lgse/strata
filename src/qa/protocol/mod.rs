// SPDX-License-Identifier: MIT

//! `tests/qa.rs` compiles this file too, so the runner rejects a malformed
//! step when it loads a scenario instead of the process rejecting it mid-run.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

mod checks;
mod chord;
mod command;
mod expr;
mod matchers;
mod offset;
mod predicate;
mod target;

pub(crate) use self::{
    checks::*, chord::*, command::*, expr::*, matchers::*, offset::*, predicate::*, target::*,
};

pub(crate) const DEFAULT_WAIT: Duration = Duration::from_millis(15_000);
/// Motion events a drag sends between its press and release.
pub(crate) const DRAG_STEPS: u32 = 16;
/// Wheel clicks a `wheel` step sends when it names no count.
pub(crate) const WHEEL_CLICKS: u32 = 3;

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Failure {
    pub(crate) predicate: Value,
    pub(crate) actual: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Reply {
    pub(crate) ok: bool,
    pub(crate) app: Value,
    pub(crate) changed: Vec<String>,
    pub(crate) criticals: Vec<String>,
    pub(crate) result: Value,
    pub(crate) failures: Vec<Failure>,
    pub(crate) error: Option<String>,
}

pub(crate) fn changed_keys(previous: Option<&Value>, current: &Value) -> Vec<String> {
    let Value::Object(current) = current else {
        return Vec::new();
    };
    let previous = previous.and_then(Value::as_object);
    current
        .iter()
        .filter(|(key, value)| previous.and_then(|previous| previous.get(*key)) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect()
}
