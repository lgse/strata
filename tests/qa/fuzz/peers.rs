// SPDX-License-Identifier: MIT

//! The runner's own steps a sequence may take besides `fs`: a drag source in another process
//! (`foreign`, `foreign_label`, `foreign_stop`), as the `content_drops` scenarios start it,
//! on the target's own display (`tests/qa/foreign_drag_source.py`). A replay hands them to the
//! scenario runner's `peer_step`, so a sequence's source is the one a scenario's is.
//!
//! A `foreign` step names its source `as` a kept target, which a `drag` from `kept` in the
//! same action drags from. A shrunk sequence can keep the drag without the step that started
//! the source; the drag is then refused on every target alike, which the replays agree on.

use serde_json::{Map, Value, json};

use super::observe::Outcome;
use crate::{
    Application,
    drive::Context,
    peer_steps::{PeerError, Peers, peer_step},
};

/// The runner's steps a sequence may take.
pub(crate) const STEPS: [&str; 3] = ["foreign", "foreign_label", "foreign_stop"];

/// The one of [`STEPS`] that names what it starts, with `as`.
pub(crate) const NAMING: &str = "foreign";

/// Which of [`STEPS`] `step` is, if any.
pub(crate) fn kind(step: &Map<String, Value>) -> Option<&'static str> {
    STEPS.iter().copied().find(|key| step.contains_key(*key))
}

/// What a peer step did, why the target is gone when the step lost it, and the command and
/// reply to record.
pub(crate) struct Taken {
    pub(crate) outcome: Outcome,
    pub(crate) gone: Option<String>,
    pub(crate) record: (Value, Value),
}

/// Takes `step` when it is one of [`STEPS`]: `None` for any other step. A source that fails
/// to start, or a wait for its label that runs out, is the step's outcome, as a refused or a
/// failed step is the target's.
pub(crate) fn take(
    application: &mut Application,
    peers: &mut Peers,
    context: &Context,
    step: &Value,
) -> Option<Taken> {
    let object = step.as_object()?;
    kind(object)?;
    let mut notes = Vec::new();
    let result = peer_step(application, peers, context, object, &mut notes)?;
    let failed = |outcome, error: String| Taken {
        outcome,
        gone: None,
        record: (step.clone(), json!({ "ok": false, "error": error })),
    };
    Some(match result {
        Ok(Some((request, reply))) => {
            let (outcome, gone) = super::replay::answered(&reply);
            Taken {
                outcome,
                gone,
                record: (request, reply),
            }
        }
        Ok(None) => Taken {
            outcome: Outcome::Ok,
            gone: None,
            record: (step.clone(), json!({ "ok": true })),
        },
        Err(PeerError::Failed(error)) => failed(Outcome::Failed, error),
        Err(PeerError::Harness(error)) => failed(Outcome::Refused, error),
    })
}
