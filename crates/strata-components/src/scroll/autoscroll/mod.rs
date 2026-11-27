// SPDX-License-Identifier: MIT

mod collection;
mod machine;
mod shield;
mod speed;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use rune::Tx;

pub use collection::AutoscrollPointer;
pub use machine::{Start, Stop, Target};
pub use shield::{ANCHOR, KEY as SHIELD, Shield};
pub use speed::{AutoscrollSettings, speed};

use crate::contract::{Point, Rect};
use machine::Machine;

#[derive(Clone, Default)]
pub struct Autoscroll(Rc<RefCell<Machine>>);

impl Autoscroll {
    /// Nothing running.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, start: Start, settings: AutoscrollSettings) -> bool {
        self.0.borrow_mut().start(start, settings)
    }

    /// Whether one runs, or is stopping.
    pub fn active(&self) -> bool {
        self.0.borrow().active()
    }

    /// Who started the one running (`Start::owner`).
    pub fn owner(&self) -> Option<String> {
        self.0.borrow().owner().map(str::to_owned)
    }

    pub fn shown(&self) -> Option<(Point, Rect)> {
        self.0.borrow().running().map(|r| (r.anchor, r.viewport))
    }

    /// The pointer, in window points, of the one running.
    pub fn pointer(&self) -> Option<Point> {
        self.0.borrow().running().map(|r| r.pointer)
    }

    pub fn track(&self, p: Point) -> bool {
        self.0.borrow_mut().track(p)
    }

    pub fn drive(&self, now: f64, animations: bool, tx: &mut Tx) -> bool {
        self.0.borrow_mut().step(now, animations, tx)
    }

    /// Stops now, leaving each viewport where it is. True when one ran.
    pub fn stop_now(&self, tx: &mut Tx) -> bool {
        self.0.borrow_mut().stop_now(tx)
    }

    pub fn request_stop(&self) {
        self.0.borrow_mut().request_stop();
    }

    pub fn stop_gone(&self, owner: Option<&str>) -> bool {
        self.0.borrow_mut().stop_gone(owner)
    }
}

impl std::fmt::Debug for Autoscroll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autoscroll")
            .field("owner", &self.owner())
            .finish()
    }
}
