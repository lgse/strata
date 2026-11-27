// SPDX-License-Identifier: MIT

use std::time::Duration;

use serde_json::{Value, json};

use crate::Failure;

/// What one step came to, before the reply adds the application's state to it.
#[derive(Debug, Default)]
pub struct Outcome {
    pub result: Value,
    pub failures: Vec<Failure>,
    pub error: Option<String>,
}

impl Outcome {
    pub fn error(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }

    pub fn from(result: Result<(), String>) -> Self {
        result.map_or_else(Self::error, |()| Self::default())
    }

    /// `tree` is the dump of the window the step gave up on.
    pub fn timed_out(tree: String, failure: Failure, timeout: Duration) -> Self {
        Self {
            result: json!({ "tree": tree }),
            failures: vec![failure],
            error: Some(format!("timed out after {} ms", timeout.as_millis())),
        }
    }
}
