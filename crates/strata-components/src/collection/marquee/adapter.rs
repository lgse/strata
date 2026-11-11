//! The marquee on the collection element's seam (C1e): [`Marquee`] is the
//! [`MarqueeController`] a host installs, and C1e's [`Ctx`] is its machine's [`Surface`].

use rune::{El, Live, Mods};
use strata_kit::look::Look;

use super::machine::{MarqueeMachine, Outcome, Surface};
use super::settings::MarqueeSettings;
use super::view::band_view;
use crate::collection::element::{Ctx, Effect, Flow, MarqueeController};
use crate::collection::selection::{Changed, Selection};
use crate::contract::{CollectionGeometry, EntrySource, Point, Rect};

/// Where the band draws when there is nothing near the view to draw: off the content, no size.
const HIDDEN: [f32; 4] = [-64.0, -64.0, 0.0, 0.0];

/// Marquee selection, installed on a collection:
/// `state.controllers.marquee = Some(Box::new(Marquee::new()))`. Its settings are the
/// collection's [`MarqueeSettings`], if the host gives one (`Collection::settings`); a host
/// sets at least [`MarqueeSettings::animations`] from `Look::animations`, or the edge scroll
/// runs too fast under reduced motion.
///
/// The band draws in the element's marquee layer from a live rect, so a move or an auto-scroll
/// frame that crosses no new entry runs no view: only showing and hiding the band
/// ([`Ctx::repaint`]) and selection changes do.
///
/// Used by: C2, C3, C4, and hosts.
pub struct Marquee {
    machine: MarqueeMachine,
    /// The band's drawn rect, `[x, y, w, h]` in content coordinates.
    rect: Live<[f32; 4]>,
}

impl Marquee {
    /// A marquee with no band open.
    pub fn new() -> Self {
        Self {
            machine: MarqueeMachine::new(),
            rect: Live::new([0.0; 4]),
        }
    }

    /// Runs `f` on the machine with `cx` as its surface, then moves the drawn band.
    fn with<R>(
        &mut self,
        cx: &mut Ctx<'_>,
        f: impl FnOnce(&mut MarqueeMachine, &mut On) -> R,
    ) -> R {
        let settings = cx
            .settings::<MarqueeSettings>()
            .copied()
            .unwrap_or_default();
        let r = f(&mut self.machine, &mut On { cx, settings });
        // With nothing to draw near the view, the band goes off it: above the content's top,
        // with no size, so a band left from before draws nowhere.
        let drawn = self.machine.drawn().map_or(HIDDEN, <[f32; 4]>::from);
        self.rect.set(drawn);
        r
    }

    /// The band's drawn rect now, `[x, y, w, h]` in content coordinates.
    #[cfg(test)]
    pub(super) fn drawn_rect(&self) -> [f32; 4] {
        self.rect.get()
    }
}

impl std::fmt::Debug for Marquee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Marquee")
            .field("machine", &self.machine)
            .field("rect", &self.rect.get())
            .finish()
    }
}

impl Default for Marquee {
    fn default() -> Self {
        Self::new()
    }
}

impl MarqueeController for Marquee {
    fn begin(&mut self, at: Point, mods: Mods, cx: &mut Ctx<'_>) {
        self.with(cx, |m, s| m.begin(at, mods, s));
    }

    fn moved(&mut self, at: Point, cx: &mut Ctx<'_>) {
        self.with(cx, |m, s| m.moved(at, s));
    }

    fn end(&mut self, cx: &mut Ctx<'_>) {
        self.with(cx, |m, s| m.end(s));
    }

    fn cancel(&mut self, cx: &mut Ctx<'_>) -> bool {
        self.with(cx, |m, s| m.cancel(s))
    }

    fn active(&self) -> bool {
        self.machine.is_active()
    }

    fn band(&self) -> Option<Rect> {
        self.machine.band()
    }

    fn overlay(&self, look: &Look) -> Option<El> {
        self.machine
            .is_active()
            .then(|| band_view(look, &self.rect))
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        if self.with(cx, |m, s| m.tick(s)) {
            Flow::Handled
        } else {
            Flow::Pass
        }
    }

    fn reset(&mut self) {
        self.machine.reset();
    }

    fn yield_scroll(&mut self) -> bool {
        self.machine.yield_scroll()
    }
}

/// C1e's context as the machine's surface.
struct On<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    settings: MarqueeSettings,
}

impl Surface for On<'_, '_> {
    fn source(&self) -> &dyn EntrySource {
        self.cx.source
    }

    fn geometry(&self) -> &dyn CollectionGeometry {
        self.cx.geometry
    }

    fn visible(&self) -> Rect {
        self.cx.visible
    }

    fn now(&self) -> f64 {
        self.cx.now
    }

    fn content_extent(&self) -> f32 {
        self.cx.content_extent()
    }

    fn settings(&self) -> MarqueeSettings {
        self.settings
    }

    fn selection(&self) -> &Selection {
        self.cx.selection()
    }

    fn apply(
        &mut self,
        op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed,
    ) -> Changed {
        self.cx.apply(|sel, src| op(sel, src))
    }

    fn emit(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Glide { y, spring } | Outcome::StopGlide { y, spring } => {
                self.cx.emit(Effect::Glide { y, spring });
            }
            Outcome::StopScroll { y } => self.cx.emit(Effect::ScrollTo { y, glide: false }),
            Outcome::WakeAfter(d) => self.cx.emit(Effect::WakeAfter(d)),
            Outcome::Repaint => self.cx.repaint(),
        }
    }
}
