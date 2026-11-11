//! The two backend-neutral pieces of a drag: the press guard on a drag source, and which
//! target a drag over a window lands on.
//!
//! # The press guard, and Rune's capture (U10)
//!
//! A drag source is an `on_drag` element: Rune's `drag_out` starts nothing on Linux (draft
//! U1, not filed). Once the system drag starts, the compositor or X server has the pointer,
//! and Rune's capture of the press is left behind:
//!
//! - **Wayland:** winit gets a pointer leave and never the release, so Rune stays pressing
//!   and later moves reach `on_drag` with no button held (S2, Q3). Rune has no public way to
//!   end the capture (draft U10, not filed). The workaround: P1 sends Escape through
//!   `Tx::send_key`, which Rune handles as "Escape during a drag cancels it"
//!   (`Drag::cancelled`'s documented cause): the element hears `Up` with `cancelled`, the
//!   capture ends, and the guard swallows that `Up`.
//!
//!   An Escape that finds no capture reaches the app's key handlers, so it is sent only when
//!   the capture is known to be held:
//!   - Not from the `Move` that crossed the threshold: a fast flick brings that move and the
//!     release in one batch, and Rune ends the capture at the release's input, before any
//!     handler runs (review round 1, B2).
//!   - Only once the compositor proves it took the pointer, by the first of: the drag's own
//!     `wl_data_device.enter` on one of the app's windows, or a target answering the drag
//!     (`wl_data_source.target` or `action`), for a drag that starts or stays outside them
//!     (review round 2, R2-B2). A compositor takes a drag only while the button is held
//!     (wlroots checks the press serial), and then winit never hears the release.
//!   - In that update, P1 first dispatches P0's queue and checks that the drag hasn't dropped
//!     or ended: a focus change at the drop (sway's bounce) follows those in the protocol
//!     stream. Nothing winit reads can run until the update returns, and Rune handles the
//!     sent key before it does.
//!   - Only while the element the press captured is live at the path it had when the drag
//!     started (review round 2, R2-B1): Rune's `cancel_drag` finds it by that path, and
//!     skips exiting subtrees. P1 records the path by a pick when the drag starts, and sends
//!     the Escape from a pick at the press point (which Rune found inside the element's
//!     visible box) that runs against the same tree, just before Rune handles the key.
//!   - Only for a press the guard saw go down, and once.
//!
//!   With no proof at all (the drag starts over no surface, and no target answers it), P1
//!   sends nothing: Rune's capture is held until the window's next press or focus change.
//!   Nothing ends it later on a timer (review round 3, B1): Rune drops a capture without an
//!   `Up` when a press replaces it or a modal mounts, and an Escape after that would reach
//!   the app.
//!
//!   What remains is listed in the module docs, "Rune's capture on Wayland".
//! - **X11:** the source is grab-less and winit keeps its implicit grab, so Rune hears every
//!   move and the real release. The guard swallows them, and the real release also drops
//!   the drag (a backstop for a release the XI2 events missed, B1). A cancelled `Up` (the
//!   user's Escape, or focus lost) cancels the drag.

use std::any::Any;
use std::rc::Rc;

use rune::{Drag, Mods, Phase};

use super::api::{Actions, DragAction, DropOffer, DropPoint, Hover};
use super::negotiate;
use super::registry::Registry;

/// How the platform ends Rune's capture of a press that became a system drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    /// Wayland: P1 sends Escape; a cancelled `Up` follows, which is ours.
    Escape,
    /// X11: Rune hears the real release; a cancelled `Up` before it cancels the drag.
    RealUp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Pressed {
        x: f32,
        y: f32,
    },
    /// A system drag runs from the press. `escape`: P1 may still end Rune's capture with an
    /// Escape (Wayland, a press the guard saw go down, none sent yet).
    Started {
        release: Release,
        escape: bool,
    },
}

/// What a source's `on_drag` does with a phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Not the system drag's: the element's own handler hears it.
    Pass,
    /// The pointer crossed the threshold: start the drag.
    Start,
    /// The system drag's: swallowed.
    Own,
    /// X11: the press was cancelled under a running drag (Escape, focus): cancel it.
    Cancel,
    /// X11: the real release under a running drag: it drops, if the XI2 events missed it.
    Released,
}

#[derive(Debug)]
pub(crate) struct Guard {
    state: State,
}

impl Default for Guard {
    fn default() -> Self {
        Guard { state: State::Idle }
    }
}

/// One phase of a press, as the guard reads it from Rune's `Drag`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Step {
    pub(crate) phase: Phase,
    /// The pointer, in window points.
    pub(crate) at: [f32; 2],
    pub(crate) cancelled: bool,
}

impl Step {
    pub(crate) fn of(d: &Drag) -> Step {
        Step {
            phase: d.phase,
            at: d.window_point(),
            cancelled: d.cancelled(),
        }
    }
}

impl Guard {
    /// A source element's phase, with its threshold in points.
    pub(crate) fn phase(&mut self, d: Step, threshold: f32) -> Decision {
        if let (Phase::Move, State::Pressed { x, y }) = (d.phase, self.state) {
            let [px, py] = d.at;
            return if (px - x).hypot(py - y) > threshold {
                Decision::Start
            } else {
                Decision::Pass
            };
        }
        self.owns(d)
    }

    /// The drag started from the press the guard holds (or, for an app's own `on_drag`
    /// calling `start_drag`, from whatever press is down).
    pub(crate) fn started(&mut self, release: Release) {
        let pressed = matches!(self.state, State::Pressed { .. });
        self.state = State::Started {
            release,
            escape: release == Release::Escape && pressed,
        };
    }

    /// Whether [`Guard::take_escape`] would allow the Escape, without spending it.
    pub(crate) fn may_escape(&self) -> bool {
        matches!(self.state, State::Started { escape: true, .. })
    }

    /// Whether P1 may send its Escape now: a Wayland drag from a press the guard saw go
    /// down, whose `Up` hasn't come, and no Escape sent yet. Taking it spends it.
    pub(crate) fn take_escape(&mut self) -> bool {
        match &mut self.state {
            State::Started { escape, .. } if *escape => {
                *escape = false;
                true
            }
            _ => false,
        }
    }

    /// For [`super::Dnd::owns`]: whether `d` belongs to the system drag.
    pub(crate) fn owns(&mut self, d: Step) -> Decision {
        match (d.phase, self.state) {
            (Phase::Down, _) => {
                let [x, y] = d.at;
                self.state = State::Pressed { x, y };
                Decision::Pass
            }
            (Phase::Move, State::Started { .. }) => Decision::Own,
            (Phase::Move, _) => Decision::Pass,
            (Phase::Up, State::Started { release, .. }) => {
                self.state = State::Idle;
                match (release, d.cancelled) {
                    (Release::RealUp, true) => Decision::Cancel,
                    (Release::RealUp, false) => Decision::Released,
                    (Release::Escape, _) => Decision::Own,
                }
            }
            (Phase::Up, _) => {
                self.state = State::Idle;
                Decision::Pass
            }
        }
    }
}

/// Where a drag over a window lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Landing {
    /// No target takes it here.
    Nowhere,
    /// `key` takes it with this action, reading `accept` (the in-process token for a local
    /// payload).
    Accepting {
        key: String,
        action: DragAction,
        accept: String,
    },
    /// `key` refuses it and stops it ([`DropTarget::on_reject`](super::DropTarget::on_reject)).
    Rejecting { key: String },
}

impl Landing {
    pub(crate) fn key(&self) -> Option<&str> {
        match self {
            Landing::Nowhere => None,
            Landing::Accepting { key, .. } | Landing::Rejecting { key } => Some(key),
        }
    }

    pub(crate) fn hover(&self) -> Hover {
        match self {
            Landing::Nowhere => Hover::Left,
            Landing::Accepting { action, .. } => Hover::Accepting(*action),
            Landing::Rejecting { .. } => Hover::Rejecting,
        }
    }
}

/// The actions to negotiate with: the source's, or copy while they aren't known yet
/// (Wayland says just after the drag enters).
pub(crate) fn allowed(offer: &DropOffer) -> Actions {
    if offer.actions.is_empty() {
        Actions::COPY
    } else {
        offer.actions
    }
}

/// The target of the drag at a hit test's `path`: the innermost that takes it.
#[cfg(test)]
pub(crate) fn land<S: 'static>(
    registry: &Registry<S>,
    path: Option<&str>,
    s: &S,
    offer: &DropOffer,
    local: Option<(&str, &Rc<dyn Any>)>,
    mods: Option<Mods>,
) -> Landing {
    land_at(registry, path, s, offer, local, mods, None)
}

/// [`land`], with where the drag is in window points, for targets that ask by point
/// ([`DropTarget::action_at`](super::DropTarget::action_at)).
pub(crate) fn land_at<S: 'static>(
    registry: &Registry<S>,
    path: Option<&str>,
    s: &S,
    offer: &DropOffer,
    local: Option<(&str, &Rc<dyn Any>)>,
    mods: Option<Mods>,
    at: Option<(f32, f32)>,
) -> Landing {
    let Some(path) = path else {
        return Landing::Nowhere;
    };
    let payload = local.map(|(_, p)| p);
    for key in registry.along(path) {
        let Some(entry) = registry.get(&key) else {
            continue;
        };
        let t = &entry.target;
        let stops = t.on_reject.is_some();
        if !t.takes(offer, payload) {
            if stops {
                return Landing::Rejecting { key };
            }
            continue;
        }
        let pick = match &t.action_at {
            Some(f) => {
                let (x, y) = at.unwrap_or_default();
                let [rx, ry, ..] = entry.rect.get();
                let point = DropPoint {
                    x: x - rx,
                    y: y - ry,
                    window_x: x,
                    window_y: y,
                };
                Some(f(s, offer, mods, point))
            }
            None => t.action.as_ref().map(|f| f(s, offer, mods)),
        };
        let action = negotiate::drop_action(pick, allowed(offer), mods);
        let accept = match local {
            Some((token, p)) if t.takes_local(Some(p)) => Some(token.to_owned()),
            _ => t.mimes.iter().find(|m| offer.offers(m)).cloned(),
        };
        match (action, accept) {
            (Some(action), Some(accept)) => {
                return Landing::Accepting {
                    key,
                    action,
                    accept,
                };
            }
            _ if stops => {
                return Landing::Rejecting { key };
            }
            _ => continue,
        }
    }
    Landing::Nowhere
}

#[cfg(test)]
mod tests;
