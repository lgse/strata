//! [`Shield`]: what a running autoscroll mounts over the window. A modal layer that takes every
//! press and Escape (so the press that stops it does nothing else), follows the pointer, shows
//! the scroll cursor over the listing and draws the anchor.

use std::rc::Rc;
use std::time::Duration;

use rune::{Cursor, El, PointerButton, Tx, col};
use strata_kit::look::Look;
use strata_kit::slots::scroll as tokens;

use super::{Autoscroll, Stop};
use crate::contract::Point;

/// The shield's key; the anchor's is [`ANCHOR`], under it.
pub const KEY: &str = "autoscroll-shield";

/// The anchor's key, under [`KEY`].
pub const ANCHOR: &str = "autoscroll-anchor";

type GetFn<S> = Rc<dyn Fn(&S) -> &Autoscroll>;
type LookFn<S> = Rc<dyn Fn(&S) -> &Look>;
type StopFn<S> = Rc<dyn Fn(&mut S, Stop)>;

/// The shield's builder. The host mounts what [`Shield::build`] returns as the last child of a
/// root that fills its window from the top-left (window points are its points), on every
/// view: it is `None` unless an autoscroll runs.
///
/// While mounted it is a modal (`El::modal`), as Strata's capture-phase gestures are:
///
/// - any press, with any button, anywhere in the window stops the autoscroll and goes no
///   further, so it neither selects, opens nor starts a band;
/// - Escape stops it, whatever has focus, and reaches nothing else;
/// - the pointer's moves steer it, wherever they are;
/// - the pointer over the listing shows [`Cursor::Move`] (Rune has no all-scroll cursor).
///
/// See the [scroll module's deviations](crate::scroll#deviations) for what the modal costs.
///
/// Used by: hosts (X1's window root), the gallery.
pub struct Shield<S> {
    get: GetFn<S>,
    look: LookFn<S>,
    label: String,
    on_stop: Option<StopFn<S>>,
}

impl<S: 'static> Shield<S> {
    /// The shield for the window's autoscroll, which `get` returns, drawn with `look`.
    pub fn new(
        get: impl Fn(&S) -> &Autoscroll + 'static,
        look: impl Fn(&S) -> &Look + 'static,
    ) -> Self {
        Self {
            get: Rc::new(get),
            look: Rc::new(look),
            label: "Autoscroll".to_owned(),
            on_stop: None,
        }
    }

    /// What assistive technology reads for it while it runs: "Autoscroll" by default (the host
    /// passes its translation).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Hears the autoscroll stop by Escape or a press: an Escape also cancels an armed 10xer
    /// chord (the host's).
    pub fn on_stop(mut self, f: impl Fn(&mut S, Stop) + 'static) -> Self {
        self.on_stop = Some(Rc::new(f));
        self
    }

    /// `el` taking every press, with any button: each stops the autoscroll and goes no further.
    fn presses(&self, el: El) -> El {
        let (get, stop) = (self.get.clone(), self.on_stop.clone());
        let press = Rc::new(move |s: &mut S, button: PointerButton, tx: &mut Tx| {
            if get(s).stop_now(tx)
                && let Some(f) = &stop
            {
                f(s, Stop::Press(button));
            }
        });
        let secondary = press.clone();
        el.pointer_buttons([
            PointerButton::Middle,
            PointerButton::Back,
            PointerButton::Forward,
            // Rune gives every other button one bit: this takes them all.
            PointerButton::Other(0),
        ])
        .on_drag(move |s: &mut S, d: &mut rune::Drag, tx: &mut Tx| {
            if d.phase == rune::Phase::Down {
                press(s, tx.button(), tx);
            } else {
                tx.live_only();
            }
        })
        .on_secondary_press(move |s: &mut S, _p, tx: &mut Tx| {
            secondary(s, PointerButton::Secondary, tx);
        })
    }

    /// The shield while an autoscroll runs, else `None`.
    pub fn build(self, s: &S) -> Option<El> {
        let auto = (self.get)(s);
        let (anchor, viewport) = auto.shown()?;
        let look = (self.look)(s);

        let size = tokens::anchor_size(look);
        let marker = col()
            .key(ANCHOR)
            .absolute(
                anchor.x - viewport.x - size / 2.0,
                anchor.y - viewport.y - size / 2.0,
            )
            .size(size, size)
            .radius(size / 2.0)
            .fill(tokens::anchor_fill(look))
            .border(
                tokens::anchor_border_width(look),
                tokens::anchor_border(look),
            )
            .shadow(
                tokens::anchor_shadow_offset(look),
                tokens::anchor_shadow_blur(look),
                tokens::anchor_shadow(look),
            );
        // Over the listing: the scroll cursor, and the anchor clipped to it. Rune takes the
        // cursor from the element a press would go to, so this region takes presses too.
        let listing = self
            .presses(col())
            .absolute(viewport.x, viewport.y)
            .size(viewport.w.max(0.0), viewport.h.max(0.0))
            .clip_soft(0.0)
            .cursor(Cursor::Move)
            // The dialog is what assistive technology hears; this is pointer-only.
            .a11y_hidden()
            .child(marker);

        let (get, stop) = (self.get.clone(), self.on_stop.clone());
        let dismiss = move |s: &mut S| {
            get(s).request_stop();
            if let Some(f) = &stop {
                f(s, Stop::Escape);
            }
        };
        let mover = auto.clone();
        // The presses are a layer's under the dialog, not the dialog's own: a secondary-press
        // handler on the dialog would give it a context-menu action for assistive technology.
        let window = self
            .presses(col())
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .a11y_hidden();
        Some(
            col()
                .key(KEY)
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                // A modal reads as a dialog, and hides the window behind it from assistive
                // technology while it runs.
                .label(self.label.clone())
                .modal(false, dismiss)
                .on_pointer_move_any(move |p, tx| {
                    tx.live_only();
                    if mover.track(Point::new(p.window_x, p.window_y)) {
                        tx.wake_after(Duration::ZERO);
                    }
                })
                .child(window)
                .child(listing),
        )
    }
}
