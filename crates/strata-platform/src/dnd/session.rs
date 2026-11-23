// SPDX-License-Identifier: MIT

use std::any::Any;
use std::rc::Rc;

use rune::{Drag, Mods, Phase};

use super::api::{Actions, DragAction, DropOffer, DropPoint, Hover};
use super::negotiate;
use super::registry::Registry;

/// How the platform ends Rune's capture of a press that became a system drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    Escape,
    /// X11: Rune hears the real release; a cancelled `Up` before it cancels the drag.
    RealUp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Pressed { x: f32, y: f32 },
    Started { release: Release, escape: bool },
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
            _ => t
                .mimes
                .iter()
                .find_map(|m| offer.offered(m))
                .map(str::to_owned),
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
