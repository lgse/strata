// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use rune::{El, Tx, WheelEvent, el};

use super::Target;
use crate::contract::Point;

type CloseFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type ListingsFn<S> = Rc<dyn Fn(&S, Point) -> Vec<Target>>;

/// Dismiss scrim for open panels that forwards outside wheel ticks to underlying listings.
/// Only turns vertical Shift+wheel ticks sideways because Rune already turns mouse-wheel
/// Shift ticks; panels need no wheel handler of their own since Rune chains wheel scrolling
/// up ancestors only (`anomalyco/rune#1137`).
pub struct OutsideWheel<S> {
    close: CloseFn<S>,
    listings: Option<ListingsFn<S>>,
}

impl<S> Clone for OutsideWheel<S> {
    fn clone(&self) -> Self {
        Self {
            close: self.close.clone(),
            listings: self.listings.clone(),
        }
    }
}

impl<S: 'static> OutsideWheel<S> {
    /// The outside of a panel that `close` closes.
    pub fn new(close: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        Self {
            close: Rc::new(close),
            listings: None,
        }
    }

    pub fn listings(mut self, f: impl Fn(&S, Point) -> Vec<Target> + 'static) -> Self {
        self.listings = Some(Rc::new(f));
        self
    }

    pub fn scrim(&self, key: impl Into<String>) -> El {
        let (press, wheel) = (self.clone(), self.clone());
        let pending = Pending::default();
        el().key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .on_background_press_tx(move |s: &mut S, tx: &mut Tx| (press.close)(s, tx))
            .on_wheel(move |s: &mut S, e: WheelEvent, tx: &mut Tx| {
                wheel.tick(s, e, &pending, tx);
                true
            })
    }

    fn tick(&self, s: &mut S, e: WheelEvent, pending: &Pending, tx: &mut Tx) {
        let (mut dx, mut dy) = (e.dx, e.dy);
        if e.mods.shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        let targets = self
            .listings
            .as_ref()
            .map(|f| f(s, Point::new(e.x, e.y)))
            .unwrap_or_default();
        (self.close)(s, tx);
        for t in targets {
            let p = t.feed.get();
            let from = pending.from(&t.path, p.offset);
            let along = |on: bool, i: usize, d: f32| {
                if on {
                    (from[i] - d).clamp(0.0, p.max[i].max(0.0))
                } else {
                    from[i]
                }
            };
            let to = [along(t.x, 0, dx), along(t.y, 1, dy)];
            if to != from {
                pending.set(&t.path, p.offset, to);
                tx.scroll_to_instant(t.path.clone(), to[0], to[1]);
            }
        }
    }
}

#[derive(Clone, Default)]
struct Pending(Rc<RefCell<Vec<Asked>>>);

struct Asked {
    path: String,
    /// The feed's offset when it was asked, and what it was asked to move to.
    seen: [f32; 2],
    to: [f32; 2],
}

impl Pending {
    fn from(&self, path: &str, offset: [f32; 2]) -> [f32; 2] {
        self.0
            .borrow()
            .iter()
            .find(|a| a.path == path && a.seen == offset)
            .map_or(offset, |a| a.to)
    }

    fn set(&self, path: &str, seen: [f32; 2], to: [f32; 2]) {
        let mut asked = self.0.borrow_mut();
        asked.retain(|a| a.path != path);
        asked.push(Asked {
            path: path.to_owned(),
            seen,
            to,
        });
    }
}
