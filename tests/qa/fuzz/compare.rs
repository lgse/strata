// SPDX-License-Identifier: MIT

//! The comparator: two streams of observations of one sequence, read step by step until a
//! field outside the map and the mask differs.

use serde_json::{Value, json};

use super::{
    observe::{Observation, Outcome},
    project::{CRITICALS, Difference, INVARIANTS, RESULT, differences, pattern_of},
    replay::Stream,
    rules::Ignored,
};

/// Where two streams first part: the step, the action that was just taken, and every field
/// that differs there, the first one first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Divergence {
    pub(crate) step: usize,
    pub(crate) action: Value,
    pub(crate) fields: Vec<Difference>,
}

impl Divergence {
    /// The first differing field.
    pub(crate) fn field(&self) -> &str {
        self.fields
            .first()
            .map_or("", |difference| difference.field.as_str())
    }

    /// The first differing field with its list indexes taken out.
    pub(crate) fn signature(&self) -> String {
        pattern_of(self.field())
    }

    /// Whether this is the divergence `other` is, shown by other steps: the same field
    /// first, after the same action, between the same two values, whichever replay had
    /// which. For a GLib warning the values are its messages. The shrinker keeps to this,
    /// so that it cannot slide to another difference that happens to be on the same field.
    pub(crate) fn is(&self, other: &Self) -> bool {
        let (Some(ours), Some(theirs)) = (self.fields.first(), other.fields.first()) else {
            return false;
        };
        let same_values = (&ours.left, &ours.right) == (&theirs.left, &theirs.right)
            || (&ours.left, &ours.right) == (&theirs.right, &theirs.left);
        self.signature() == other.signature() && self.action == other.action && same_values
    }

    /// The title a `divergence` issue takes: the area, the first differing field and the
    /// last action, so a duplicate is found by search. A fault both replays share does not
    /// differ, and is titled for what it is.
    pub(crate) fn title(&self, area: &str) -> String {
        let shared = self
            .fields
            .first()
            .is_some_and(|difference| difference.left == difference.right);
        format!(
            "[fuzz {area}] {} {} {}",
            self.signature(),
            if shared { "after" } else { "differs after" },
            self.action
        )
    }

    pub(crate) fn report(&self) -> Value {
        json!({
            "step": self.step,
            "action": self.action,
            "field": self.field(),
            "fields": self.fields.iter().map(|difference| json!({
                "field": difference.field,
                "left": difference.left,
                "right": difference.right,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The first step at which the streams differ on a field `ignored` does not cover, or at
/// which a target logged a GLib warning, broke an invariant or was lost: those are
/// divergences in themselves, whether or not the other replay did the same.
///
/// A step that did not come to rest on either side is compared by its outcome, its
/// criticals and its invariants alone: nothing is compared while the application is still moving. A stream
/// that ends early differs from the other at the step it lost.
pub(crate) fn compare(left: &Stream, right: &Stream, ignored: &Ignored) -> Option<Divergence> {
    for (ours, theirs) in left.observations.iter().zip(&right.observations) {
        let at_rest = ours.settled && theirs.settled;
        let mut fields: Vec<Difference> = differences(&ours.value, &theirs.value)
            .into_iter()
            .filter(|difference| at_rest || is_verdict(&difference.field))
            .filter(|difference| !ignored.covers(&difference.field))
            .collect();
        if fields.is_empty() {
            fields.extend(shared_fault(ours, theirs));
        }
        if !fields.is_empty() {
            return Some(Divergence {
                step: ours.step,
                action: ours.action.clone(),
                fields,
            });
        }
    }
    let shared = left.observations.len().min(right.observations.len());
    let longer = if left.observations.len() > shared {
        left
    } else {
        right
    };
    let extra = longer.observations.get(shared)?;
    let present = |stream: &Stream| {
        stream
            .observations
            .get(shared)
            .map_or(Value::Null, |observation: &Observation| {
                observation.value[RESULT].clone()
            })
    };
    Some(Divergence {
        step: extra.step,
        action: extra.action.clone(),
        fields: vec![Difference {
            field: RESULT.to_owned(),
            left: present(left),
            right: present(right),
        }],
    })
}

/// What both replays did alike and neither should have: a GLib warning, an invariant
/// broken, or a lost target.
fn shared_fault(ours: &Observation, theirs: &Observation) -> Option<Difference> {
    let field = if ours.criticals() > 0 {
        CRITICALS
    } else if ours.broken() > 0 {
        INVARIANTS
    } else if ours.outcome() == Outcome::Crashed.name() {
        RESULT
    } else {
        return None;
    };
    Some(Difference {
        field: field.to_owned(),
        left: ours.value[field].clone(),
        right: theirs.value[field].clone(),
    })
}

/// A field that says how the step ended, which does not depend on when it was read.
fn is_verdict(field: &str) -> bool {
    field == RESULT || field == CRITICALS || field == INVARIANTS
}
