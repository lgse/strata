// SPDX-License-Identifier: MIT

mod commands;
mod pixels;
mod predicates;
mod snapshot;
mod targeting;

use super::Predicate;

fn predicate(json: serde_json::Value) -> Predicate {
    serde_json::from_value(json).expect("the predicate should parse")
}
