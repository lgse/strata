// SPDX-License-Identifier: MIT

//! A divergence put in on purpose, to prove the comparator and the shrinker where there is
//! only one target to compare. The harness changes one field of one observation of the
//! second replay; the application is not touched.
//!
//! The change waits for a step and for a state: after the step `after`, when the observation
//! before it held every `while` field. So the divergence depends on what the application
//! really did, and a shrunk sequence has to keep the steps that bring it to that state.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::{
    project::{set_at, value_at},
    sequence::strip,
};
use crate::STEP_WHEN;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Perturbation {
    /// The step, without its `when`, whose observation is changed.
    pub(crate) after: Value,
    /// What the observation before that step must hold, by field.
    #[serde(default, rename = "while")]
    pub(crate) held: BTreeMap<String, Value>,
    /// The field that is changed, and what it is changed to.
    pub(crate) field: String,
    pub(crate) value: Value,
}

impl Perturbation {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|error| {
            format!(
                "a perturbation is {{\"after\":STEP,\"while\":{{FIELD:VALUE}},\"field\":FIELD,\
                 \"value\":VALUE}}: {error}"
            )
        })
    }

    /// Changes `observed` when this is the step and the state it waits for. A step that was
    /// not sent changes nothing.
    pub(crate) fn apply(&self, step: &Value, before: &Value, observed: &mut Value) -> bool {
        strip(step, STEP_WHEN) == self.after
            && self
                .held
                .iter()
                .all(|(field, value)| value_at(before, field) == Some(value))
            && set_at(observed, &self.field, self.value.clone())
    }
}
