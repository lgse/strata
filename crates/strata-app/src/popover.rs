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
//! - **Escape closes it**, and the keyboard goes back to the button that opened it.
//! - **A press outside closes it and goes no further**: a press on a tile selected nothing, a
//!   press on the sidebar's Home opened nothing. The keyboard goes back to the button.
//! - **A wheel outside closes it**, and scrolls the listing when it is over the listing (GTK's
//!   `listing_ancestor` in `src/ui/scrolling/popover.rs`); over the sidebar or other chrome it
//!   scrolls nothing. The keyboard goes back to the button.
//!
//! Rune's `El::modal` would make everything outside inert, so a wheel there could neither
//! close it nor scroll (finding lgse/strata#2341, Rune packet entry 104). So the surface is not
//! modal: it lies over a full-window scrim that is hidden from assistive technology, takes a
//! press, and takes a wheel unless it is over [`Plain::listing`], where it returns false and
//! Rune's fallback scrolls the viewport under the point (`scroll_at_axis`, which looks past
//! what covers the point).

use rune::{El, Key, KeyEvent, Tx, WheelEvent, el};
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
    /// The keys of its Tab stops, in order: Tab and Shift+Tab cycle through them.
    pub(crate) stops: Vec<&'static str>,
    /// Where the listing is, in window points: a wheel over it scrolls it as well as closing
    /// the popover.
    pub(crate) listing: Option<[f32; 4]>,
    /// Closes the popover: the part's own state.
    pub(crate) close: fn(&mut App),
}

impl Plain {
    /// Pushes the scrim and the surface around `body` onto the layer.
    pub(crate) fn build(self, look: &Look, body: El, frame: &mut Frame<'_, App>) {
        let kit = &look.kit;
        frame
            .layer
            .push(scrim(self.key, self.anchor, self.listing, self.close));
        let (close, stops) = (self.close, self.stops);
        let body = body
            .key(self.key)
            .role("dialog")
            .label(self.label)
            .focus_return(self.anchor)
            .on_key(move |app: &mut App, event: &KeyEvent, tx: &mut Tx| {
                keyed(app, event, tx, (close, &stops))
            })
            // A press or a wheel on the surface's own blank parts stays with it.
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

/// The full-window layer under the surface: a press closes the popover and goes no further; a
/// wheel closes it, and goes on to the listing when it is over it.
fn scrim(key: &str, anchor: &'static str, listing: Option<[f32; 4]>, close: fn(&mut App)) -> El {
    el().key(format!("{key}-scrim"))
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .a11y_hidden()
        .on_press(move |app: &mut App| close(app))
        .on_wheel(move |app: &mut App, wheel: WheelEvent, tx: &mut Tx| {
            close(app);
            // A wheel passed on to the listing leaves the handler's change unseen: the focus
            // request, which GTK's close makes too, has Rune build the window again.
            tx.focus(anchor, true);
            !listing.is_some_and(|rect| inside(rect, wheel.x, wheel.y))
        })
}

fn inside([x, y, w, h]: [f32; 4], px: f32, py: f32) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// Escape closes the popover; Tab and Shift+Tab move through its stops and wrap.
fn keyed(
    app: &mut App,
    event: &KeyEvent,
    tx: &mut Tx,
    (close, stops): (fn(&mut App), &[&'static str]),
) -> bool {
    match event.key {
        Key::Escape => {
            close(app);
            true
        }
        Key::Tab if !stops.is_empty() => {
            let at = tx
                .focused()
                .and_then(|focused| stops.iter().position(|stop| *stop == focused));
            let count = stops.len();
            let next = match (at, event.mods.shift) {
                (Some(at), false) => (at + 1) % count,
                (Some(at), true) => (at + count - 1) % count,
                (None, false) => 0,
                (None, true) => count - 1,
            };
            tx.focus(stops[next], true);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
