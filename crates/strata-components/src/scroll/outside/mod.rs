//! [`OutsideWheel`]: an open panel's (Sort by, Appearance, Thumbnail size) outside layer, which
//! a wheel tick closes the panel through and then scrolls the listing under the pointer, as
//! Strata's `ui::scrolling::popover::dismiss_on_outside_scroll` does.

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use rune::{El, Tx, WheelEvent, el};

use super::Target;
use crate::contract::Point;

type CloseFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type ListingsFn<S> = Rc<dyn Fn(&S, Point) -> Vec<Target>>;

/// The outside of an open panel: a press there closes it, and a wheel tick there closes it and
/// scrolls the listing under the pointer, and nothing else, by the tick's own step.
///
/// The host mounts [`OutsideWheel::scrim`] in its root layer under the panel, in place of the
/// panel's own outside-press scrim (rune-kit `Popover::on_close`'s), and wraps the panel's
/// surface in [`OutsideWheel::surface`]:
///
/// ```ignore
/// let outside = OutsideWheel::new(|s: &mut App, _| s.sort_open = false)
///     .listings(|s: &App, at| s.listing_targets_at(at)); // the List pane: its rows and its sideways scroll
/// layer.push(outside.scrim("sort-scrim"));
/// let panel = OutsideWheel::<App>::surface(panel_body);
/// ```
///
/// - Rune hands a wheel delta to the topmost element's `on_wheel` first, so the scrim hears
///   every tick outside the panel; it takes the tick, so nothing under it scrolls by itself.
/// - The listing the tick scrolls is whichever of the host's [`Target`]s are under the pointer
///   ([`OutsideWheel::listings`]): over the sidebar or the chrome the host gives none, and the
///   tick only closes the panel. Each target moves by the tick's delta on the axes it scrolls,
///   as Rune's wheel would have moved it, and with Shift held a vertical tick moves it
///   sideways. Ticks that arrive in one frame add up, as they would on the listing itself.
/// - **Shift turns only a vertical tick** (a deviation in form, not effect): Strata swaps the
///   axes of every Shift tick, since GTK hands it the wheel unturned, but Rune's window host
///   has already turned a Shift + mouse-wheel tick sideways before the scrim hears it, and
///   swapping that again would scroll it vertically. A tick with both axes (a touchpad's) is
///   left as Rune's wheel leaves it over the listing.
/// - The rest of a touchpad gesture whose first delta closed the panel scrolls as usual.
///
/// Used by: hosts (X1's and Strata's panels), the gallery's `c24-panel` scene.
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

    /// The listing viewports under a window point, which a tick there scrolls: none over the
    /// sidebar or chrome. Without it a tick only closes the panel.
    pub fn listings(mut self, f: impl Fn(&S, Point) -> Vec<Target> + 'static) -> Self {
        self.listings = Some(Rc::new(f));
        self
    }

    /// The layer over the window, under the panel, keyed `key`. Mount it in a root layer that
    /// fills the window from its top-left (its points are window points).
    pub fn scrim(&self, key: impl Into<String>) -> El {
        let (press, wheel) = (self.clone(), self.clone());
        let pending = Pending::default();
        el().key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .on_press_tx(move |s: &mut S, tx: &mut Tx| (press.close)(s, tx))
            .on_wheel(move |s: &mut S, e: WheelEvent, tx: &mut Tx| {
                wheel.tick(s, e, &pending, tx);
                true
            })
    }

    /// The panel's surface `panel`: a tick over it that no scroll viewport inside it takes goes
    /// no further, so a panel that doesn't scroll never scrolls the listing under it (Rune
    /// scrolls the innermost viewport at the pointer, under any overlay: draft U-C24b-2).
    pub fn surface(panel: El) -> El {
        panel.on_wheel(|_: &mut S, _: WheelEvent, tx: &mut Tx| {
            tx.live_only();
            true
        })
    }

    fn tick(&self, s: &mut S, e: WheelEvent, pending: &Pending, tx: &mut Tx) {
        // Rune's window host has already turned a Shift + mouse-wheel tick (`wheel_delta` in
        // `runtime/scroll/x.rs`); a headless one (U-C24-7) is still vertical.
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

/// The offsets a scrim's ticks have asked of each listing since its feed last moved. A feed
/// moves once a frame, so ticks queued in one frame each start from the one before.
#[derive(Clone, Default)]
struct Pending(Rc<RefCell<Vec<Asked>>>);

struct Asked {
    path: String,
    /// The feed's offset when it was asked, and what it was asked to move to.
    seen: [f32; 2],
    to: [f32; 2],
}

impl Pending {
    /// Where a tick on the listing `path`, whose feed stands at `offset`, starts from: what
    /// the frame's ticks have asked of it, while the feed hasn't moved since.
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
