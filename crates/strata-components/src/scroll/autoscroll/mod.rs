//! Middle-click autoscroll: a middle press on a listing's background anchors it, and the
//! listing then scrolls toward the pointer, faster the further it moves, until Escape or any
//! press stops it. See the [scroll module](super) for the pieces and how a host wires them.

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

/// The window's autoscroll: one at a time, whichever listing started it. The host keeps one
/// per window in its state; the collection adapters ([`AutoscrollPointer`]) and the
/// [`Shield`] hold clones, which share it.
///
/// The host's poll calls [`Autoscroll::drive`] on every frame; it asks for frames only while a
/// viewport moves, so a window with an idle autoscroll, or one held in the dead zone, sleeps.
///
/// Used by: C2, C3, C4, hosts.
#[derive(Clone, Default)]
pub struct Autoscroll(Rc<RefCell<Machine>>);

impl Autoscroll {
    /// Nothing running.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts an autoscroll from `start` with `settings`, unless one runs or nothing it names
    /// can scroll. True when it started: the host's view then mounts the [`Shield`].
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

    /// The anchor and the viewport (window points) of the one running: what the shield draws.
    /// `None` once it stops, even before its last step.
    pub fn shown(&self) -> Option<(Point, Rect)> {
        self.0.borrow().running().map(|r| (r.anchor, r.viewport))
    }

    /// The pointer, in window points, of the one running.
    pub fn pointer(&self) -> Option<Point> {
        self.0.borrow().running().map(|r| r.pointer)
    }

    /// The pointer moved to `p` (window points). True when the host should ask for a frame
    /// (`tx.wake_after(ZERO)`): the autoscroll slept and now has somewhere to go.
    pub fn track(&self, p: Point) -> bool {
        self.0.borrow_mut().track(p)
    }

    /// Runs one frame at `now` (seconds, the host's clock), gliding each viewport toward the
    /// pointer, or stepping it when `animations` is off (`Look::animations`). True when what
    /// the host draws changed: a stop asked for without a transaction finished.
    pub fn drive(&self, now: f64, animations: bool, tx: &mut Tx) -> bool {
        self.0.borrow_mut().step(now, animations, tx)
    }

    /// Stops now, leaving each viewport where it is. True when one ran.
    pub fn stop_now(&self, tx: &mut Tx) -> bool {
        self.0.borrow_mut().stop_now(tx)
    }

    /// Stops on the next [`Autoscroll::drive`], for a caller with no transaction (the modal's
    /// dismiss).
    pub fn request_stop(&self) {
        self.0.borrow_mut().request_stop();
    }

    /// Stops `owner`'s autoscroll (or any, with `None`) without moving anything: its listing
    /// went to another folder, was hidden or closed. A host that hides or closes a listing
    /// calls it; [`AutoscrollPointer`]'s reset (a folder change) does too. True when one
    /// stopped.
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
