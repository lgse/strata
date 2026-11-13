// SPDX-License-Identifier: MIT

//! A plain popover, as GTK's `gtk::Popover` behaves in Strata: the Icons header's
//! "Thumbnail size" first, A8's Appearance popover next. GTK's behaviour here is driven, not
//! read (`./scripts/qa-drive.sh` on the GTK build, XTest input, 2026-11-14):
//!
//! - **It takes every key** while focus is in it, with or without modifiers: Alt+Up,
//!   Ctrl+Alt+=, Delete and F5 did nothing. The part that owns the popover answers
//!   [`Holds::Popover`] while it is open, which cuts every stage of the window's chain.
//! - **Tab and Shift+Tab cycle** through its own controls and wrap; the keyboard never leaves
//!   it. [`Plain::stops`] lists them in order.
//! - **Up and Down move** through the same controls and stop at the ends, without wrapping;
//!   on a control that takes the arrows itself (the slider, [`Plain::own_arrows`]) they are
//!   that control's.
//! - **Escape closes it**, and the keyboard goes back to the button that opened it.
//! - **A press outside, primary or secondary, closes it and goes no further**: a press on a
//!   tile selected nothing, a right-click there opened no menu, a press on the sidebar's Home
//!   opened nothing. The keyboard goes back to the button.
//! - **A wheel outside closes it**, and scrolls the listing when it is over the listing (GTK's
//!   `listing_ancestor` in `src/ui/scrolling/popover.rs`); over the sidebar or other chrome it
//!   scrolls nothing. The keyboard goes back to the button.
//! - **A press on its own blank parts** leaves the keyboard where it was: on the slider,
//!   Escape then closed it.
//! - **Focus leaving it closes it** (GTK's autohide): [`focus_left`], which the part asks in
//!   its poll.
//!
//! Rune's `El::modal` would make everything outside inert, so a wheel there could neither
//! close it nor scroll (finding lgse/strata#2341, Rune packet entry 104). So the surface is not
//! modal: it lies over a full-window scrim that is hidden from assistive technology, takes a
//! press, and takes a wheel unless it is over the listing, where it returns false and Rune's
//! fallback scrolls the viewport under the point (`scroll_at_axis`, which looks past what
//! covers the point). The listing's rect is read while the popover builds
//! (`Cx::window_layout` of [`Plain::listing`]), so the window builds again whenever it moves.
//!
//! **Nesting.** A plain popover opened from another, as A8's "Customize columns…" from its
//! Appearance popover, has its scrim over the parent's, so an outside press or wheel reaches
//! only the child's. GTK closes both then: [`Plain::parent`] closes the parent too. A press on
//! the parent's surface while the child is open lands on the child's scrim too, and so closes
//! both (not driven on GTK, which has no such pair yet). Each keeps its own [`Plain::stops`] by
//! hand, with only its own controls, and the parent's part passes its open child's to
//! [`focus_left`], so the keyboard going into the child does not close the parent.

use rune::{El, Key, KeyEvent, PointerPress, Tx, WheelEvent, el};
use rune_kit::{Placement, popover};
use strata_kit::look::Look;

use crate::{App, slots::Frame};

#[cfg(doc)]
use crate::commands::Holds;

/// A plain popover's surface, mounted in the layer while its part says it is open.
pub(crate) struct Plain {
    /// The surface's key; the scrim is keyed after it.
    pub(crate) key: &'static str,
    /// The button that opened it: the surface hangs under it, and the keyboard goes back to
    /// it when the surface goes.
    pub(crate) anchor: &'static str,
    /// The accessible name of the surface (a dialog).
    pub(crate) label: String,
    /// The keys of its controls, in order: Tab, Shift+Tab, Up and Down move through them.
    pub(crate) stops: Vec<&'static str>,
    /// Those of [`Self::stops`] that take Up and Down themselves, such as a slider.
    pub(crate) own_arrows: &'static [&'static str],
    /// The key of the shown view's listing (its element that takes the keyboard): a wheel
    /// over it scrolls it as well as closing the popover.
    pub(crate) listing: Option<&'static str>,
    /// Closes the popover: the part's own state.
    pub(crate) close: fn(&mut App),
    /// The popover this one was opened from, if any: an outside press or wheel closes it too.
    pub(crate) parent: Option<Parent>,
}

/// The popover a nested one was opened from: how to close it, and the button that opened it,
/// which takes the keyboard when both close (the child's own button goes with the parent).
#[derive(Clone, Copy)]
pub(crate) struct Parent {
    pub(crate) close: fn(&mut App),
    pub(crate) anchor: &'static str,
}

impl Plain {
    /// Pushes the scrim and the surface around `body` onto the layer.
    pub(crate) fn build(self, look: &Look, body: El, frame: &mut Frame<'_, App>) {
        let kit = &look.kit;
        let listing = self.listing.and_then(|key| frame.cx.window_layout(key));
        let closes = Closes {
            own: self.close,
            parent: self.parent.map(|parent| parent.close),
            anchor: self.parent.map_or(self.anchor, |parent| parent.anchor),
        };
        frame.layer.push(scrim(self.key, listing, closes));
        let (close, stops, own_arrows) = (self.close, self.stops, self.own_arrows);
        let body = body
            .key(self.key)
            .role("dialog")
            .label(self.label)
            .focus_return(self.anchor)
            .on_key(move |app: &mut App, event: &KeyEvent, tx: &mut Tx| {
                keyed(app, event, tx, (close, &stops, own_arrows))
            })
            // A press or a wheel on the surface's own blank parts stays with it, and leaves
            // the keyboard where it was, as GTK's does.
            .preserve_focus_on_press(true)
            .on_press(|_: &mut App| {})
            .on_wheel(|_: &mut App, _: WheelEvent, _: &mut Tx| true);
        let surface = popover::raised(kit, body, kit.radius.md);
        frame.layer.push(popover::anchored(
            kit,
            surface,
            self.anchor,
            Placement::Bottom,
            look.space.xs,
        ));
    }
}

/// What an outside press or wheel closes, and the button the keyboard goes back to: the
/// outermost popover's.
#[derive(Clone, Copy)]
struct Closes {
    own: fn(&mut App),
    parent: Option<fn(&mut App)>,
    anchor: &'static str,
}

impl Closes {
    /// Closes the popover, and its parent if it has one, and gives the outermost button the
    /// keyboard: for a nested pair neither `focus_return` would land, the child's button going
    /// with the parent.
    fn close(self, app: &mut App, tx: &mut Tx) {
        (self.own)(app);
        if let Some(parent) = self.parent {
            parent(app);
        }
        tx.focus(self.anchor, true);
    }
}

/// The full-window layer under the surface: a press, primary or secondary, closes the
/// popover and goes no further; a wheel closes it, and goes on to the listing when it is over
/// it.
fn scrim(key: &str, listing: Option<[f32; 4]>, closes: Closes) -> El {
    el().key(format!("{key}-scrim"))
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .a11y_hidden()
        .on_press_tx(move |app: &mut App, tx: &mut Tx| closes.close(app, tx))
        .on_secondary_press(move |app: &mut App, _: PointerPress, tx: &mut Tx| {
            closes.close(app, tx);
        })
        // A wheel passed on to the listing leaves the handler's change unseen; the focus
        // request `close` makes, as GTK's close does, has Rune build the window again.
        .on_wheel(move |app: &mut App, wheel: WheelEvent, tx: &mut Tx| {
            closes.close(app, tx);
            !listing.is_some_and(|rect| inside(rect, wheel.x, wheel.y))
        })
}

fn inside([x, y, w, h]: [f32; 4], px: f32, py: f32) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// Whether the keyboard has left the popover whose controls are `stops`, opened by `anchor`:
/// it is on something else, GTK's autohide. Nowhere, or on the button, as while it opens, is
/// not leaving, and neither is the keyboard in a child popover it opened, whose controls are
/// `child` (empty while none is open).
pub(crate) fn focus_left(tx: &Tx, anchor: &str, stops: &[&str], child: &[&str]) -> bool {
    tx.focused().is_some_and(|focused| {
        focused != anchor && !stops.contains(&focused) && !child.contains(&focused)
    })
}

/// Escape closes the popover; Tab and Shift+Tab move through its stops and wrap; Up and Down
/// move through them and stop at the ends, unless the control takes them itself.
fn keyed(
    app: &mut App,
    event: &KeyEvent,
    tx: &mut Tx,
    (close, stops, own_arrows): (fn(&mut App), &[&'static str], &[&'static str]),
) -> bool {
    let at = tx
        .focused()
        .and_then(|focused| stops.iter().position(|stop| *stop == focused));
    let last = stops.len().saturating_sub(1);
    let next = match event.key {
        Key::Escape => {
            close(app);
            return true;
        }
        Key::Tab if stops.is_empty() => return false,
        Key::Tab => match (at, event.mods.shift) {
            (Some(at), false) if at == last => 0,
            (Some(at), false) => at + 1,
            (Some(0), true) | (None, true) => last,
            (Some(at), true) => at - 1,
            (None, false) => 0,
        },
        Key::Up | Key::Down => {
            let Some(at) = at else {
                return false;
            };
            if own_arrows.contains(&stops[at]) {
                return false;
            }
            if event.key == Key::Down {
                (at + 1).min(last)
            } else {
                at.saturating_sub(1)
            }
        }
        _ => return false,
    };
    tx.focus(stops[next], true);
    true
}

#[cfg(test)]
mod tests;
