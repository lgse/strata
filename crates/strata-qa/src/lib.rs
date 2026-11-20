// SPDX-License-Identifier: MIT

//! The protocol of the scripted QA mode: what a step, a target and a predicate are, and how
//! each is parsed. The application and the runner in `tests/qa.rs` both use it, so the runner
//! rejects a malformed step when it loads a scenario instead of the process rejecting it mid-run.

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

pub use self::{
    checks::*, chord::*, command::*, expr::*, matchers::*, offset::*, predicate::*, target::*,
};

pub const DEFAULT_WAIT: Duration = Duration::from_millis(15_000);
/// Motion events a drag sends between its press and release.
pub const DRAG_STEPS: u32 = 16;
/// Wheel clicks a `wheel` step sends when it names no count.
pub const WHEEL_CLICKS: u32 = 3;

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Failure {
    pub predicate: Value,
    pub actual: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Reply {
    pub ok: bool,
    pub app: Value,
    pub changed: Vec<String>,
    pub criticals: Vec<String>,
    pub result: Value,
    pub failures: Vec<Failure>,
    pub error: Option<String>,
}

pub fn changed_keys(previous: Option<&Value>, current: &Value) -> Vec<String> {
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
