// SPDX-License-Identifier: MIT

//! The runner's own steps a sequence may take besides `fs`: a drag source in another process
//! (`foreign`, `foreign_label`, `foreign_stop`), as the `content_drops` scenarios start it,
//! on the target's own display (`tests/qa/foreign_drag_source.py`); and a FileChooser portal
//! request (`portal_open`, `portal_response`), as the `chooser_selection` scenarios send it.
//! A replay hands them to the scenario runner's `peer_step`, so a sequence's source or request
//! is the one a scenario's is.
//!
//! A `foreign` step names its source `as` a kept target, which a `drag` from `kept` in the
//! same action drags from. A shrunk sequence can keep the drag without the step that started
//! the source; the drag is then refused on every target alike, which the replays agree on.
//! A `foreign_label` or `foreign_stop` with no source running is skipped, as a step whose `when`
//! does not hold is: the action's source was skipped, and there is nothing to wait for or stop.
//!
//! One request is open at a time: a `portal_open` while a request awaits its answer is
//! skipped, and so is a `portal_response` before any request. A `portal_response` only
//! waits, for the answer or for none (`done`): what the portal answered is observed after
//! every step (`portal` in the observation), so a step checks none of it.

use serde_json::{Map, Value, json};

use super::observe::Outcome;
use crate::{
    Application,
    drive::Context,
    peer_steps::{PeerError, Peers, peer_step},
};

/// The runner's steps a sequence may take.
pub(crate) const STEPS: [&str; 5] = [
    "foreign",
    "foreign_label",
    "foreign_stop",
    OPENING,
    ANSWERED,
];

/// The one of [`STEPS`] that names what it starts, with `as`.
pub(crate) const NAMING: &str = "foreign";
/// The step that sends a portal request.
pub(crate) const OPENING: &str = "portal_open";
/// The step that waits on a portal request's answer.
pub(crate) const ANSWERED: &str = "portal_response";

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
    let kind = kind(object)?;
    if let Some(why) = idle(kind, peers) {
        return Some(Taken {
            outcome: Outcome::Skipped,
            gone: None,
            record: (step.clone(), json!({ "ok": true, "skipped": why })),
        });
    }
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

/// Why a step of `kind` has nothing to act on, when it has not.
fn idle(kind: &str, peers: &mut Peers) -> Option<&'static str> {
    match kind {
        NAMING => None,
        OPENING => awaiting(peers).then_some("a portal request awaits its answer"),
        ANSWERED => peers
            .portal_request()
            .is_none()
            .then_some("no portal request was sent"),
        _ => (!peers.foreign_runs()).then_some("no foreign drag source runs"),
    }
}

/// Whether a portal request awaits its answer: while one does, its chooser is open.
pub(crate) fn awaiting(peers: &mut Peers) -> bool {
    peers
        .portal_request()
        .is_some_and(|request| request.poll().is_none())
}

/// What the portal answered, as an observation holds it: nothing before a request, then
/// whether its answer came, and once it has, its status and the chosen paths, or the error
/// the request ended with.
pub(crate) fn portal_state(peers: &mut Peers) -> Value {
    let Some(request) = peers.portal_request() else {
        return Value::Null;
    };
    match request.poll() {
        None => json!({ "answered": false }),
        Some(Ok(answer)) => json!({
            "answered": true,
            "status": answer.status,
            "paths": answer
                .paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
        }),
        Some(Err(error)) => json!({ "answered": true, "error": error }),
    }
}

/// A `portal_response` in a sequence waits and checks nothing: `done`, and a `timeout_ms` or
/// `message`, but no status, names, paths, kept path, image or discard.
pub(crate) fn check_answered(step: &Map<String, Value>) -> Result<(), String> {
    let check = step[ANSWERED]
        .as_object()
        .ok_or("portal_response is an object")?;
    match check.keys().find(|key| key.as_str() != "done") {
        Some(key) => Err(format!(
            "a fuzz portal_response only waits, so it takes no {key}; the observation's portal \
             holds the answer"
        )),
        None => Ok(()),
    }
}
