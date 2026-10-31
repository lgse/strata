// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use crate::model::Location;

use super::{BrowserView, ViewState};

type Observer = Rc<dyn Fn(Option<&Location>)>;

#[derive(Default)]
pub(super) struct TabLocation {
    location: Option<Location>,
    observers: Vec<Observer>,
    next_id: u64,
    pending: Option<Pending>,
}

struct Pending {
    id: u64,
    phase: Phase,
    /// The folder a click is opening, which becomes the focused column.
    opening: Option<(usize, Location)>,
}

enum Phase {
    Pressed,
    Dispatching,
    Awaiting {
        generation: u64,
        parent: Option<(usize, Location)>,
    },
}

pub(super) struct TabLocationHold {
    state: Weak<ViewState>,
    id: u64,
}

impl TabLocationHold {
    pub(super) fn opening(&self, depth: usize, location: Location) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        if let Some(pending) = state
            .tab_location
            .borrow_mut()
            .pending
            .as_mut()
            .filter(|pending| pending.id == self.id)
        {
            pending.opening = Some((depth, location));
        }
    }

    pub(super) fn navigate(self, parent_depth: usize, action: impl FnOnce()) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let generation_before = state.browser.navigation_generation();
        let parent = state
            .browser
            .location_at(parent_depth)
            .map(|location| (parent_depth, location));
        {
            let mut location = state.tab_location.borrow_mut();
            let Some(pending) = location
                .pending
                .as_mut()
                .filter(|pending| pending.id == self.id)
            else {
                return;
            };
            // Row teardown must not end a hold while URI validation is still pending.
            pending.phase = Phase::Dispatching;
        }
        action();
        let validation = state.browser.pending_navigation_generation();
        let mut location = state.tab_location.borrow_mut();
        if let Some(pending) = location.pending.as_mut()
            && pending.id == self.id
        {
            if let Some(generation) =
                validation.filter(|generation| *generation != generation_before)
            {
                pending.phase = Phase::Awaiting { generation, parent };
            } else {
                location.pending = None;
            }
        }
        drop(location);
        state.refresh_tab_location();
    }
}

impl Drop for TabLocationHold {
    fn drop(&mut self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let mut location = state.tab_location.borrow_mut();
        if location
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == self.id && matches!(pending.phase, Phase::Pressed))
        {
            location.pending = None;
        }
        drop(location);
        state.publish_tab_location();
    }
}

impl BrowserView {
    pub(in crate::ui) fn observe_tab_location(
        &self,
        observer: impl Fn(Option<&Location>) + 'static,
    ) {
        let observer: Observer = Rc::new(observer);
        let current = {
            let mut location = self.state.tab_location.borrow_mut();
            let current = location.location.clone();
            location.observers.push(observer.clone());
            current
        };
        observer(current.as_ref());
    }
}

impl ViewState {
    pub(super) fn hold_tab_location(self: &Rc<Self>) -> TabLocationHold {
        let mut location = self.tab_location.borrow_mut();
        location.next_id += 1;
        let id = location.next_id;
        location.pending = Some(Pending {
            id,
            phase: Phase::Pressed,
            opening: None,
        });
        TabLocationHold {
            state: Rc::downgrade(self),
            id,
        }
    }

    pub(super) fn pointer_opening_depth(&self) -> Option<usize> {
        self.tab_location
            .borrow()
            .pending
            .as_ref()
            .and_then(|pending| pending.opening.as_ref())
            .map(|(depth, _)| *depth)
    }

    pub(super) fn pointer_opens(&self, depth: usize, location: &Location) -> bool {
        self.tab_location
            .borrow()
            .pending
            .as_ref()
            .and_then(|pending| pending.opening.as_ref())
            .is_some_and(|(opening_depth, opening)| *opening_depth == depth && opening == location)
    }

    pub(super) fn cancel_tab_location_hold(&self) {
        self.tab_location.borrow_mut().pending = None;
        self.publish_tab_location();
    }

    pub(super) fn refresh_tab_location(&self) {
        if self.browser.navigation_update_in_progress() {
            return;
        }
        let mut location = self.tab_location.borrow_mut();
        let finished = location.pending.as_ref().is_some_and(|pending| {
            if let Phase::Awaiting { generation, parent } = &pending.phase {
                self.browser.pending_navigation_generation() != Some(*generation)
                    || parent.as_ref().is_some_and(|(depth, location)| {
                        self.browser.location_at(*depth).as_ref() != Some(location)
                    })
            } else {
                false
            }
        });
        if finished {
            location.pending = None;
        }
        drop(location);
        self.publish_tab_location();
    }

    fn publish_tab_location(&self) {
        if self.browser.navigation_update_in_progress() {
            return;
        }
        let current = self.browser.active_location();
        let observers = {
            let mut location = self.tab_location.borrow_mut();
            if location.pending.is_some() || location.location == current {
                return;
            }
            location.location = current.clone();
            location.observers.clone()
        };
        for observer in observers {
            observer(current.as_ref());
        }
    }
}

#[cfg(test)]
mod tests;
