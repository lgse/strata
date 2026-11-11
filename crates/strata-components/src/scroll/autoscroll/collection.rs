//! [`AutoscrollPointer`]: autoscroll on C1e's collection, as a decorator over its pointer
//! controller (C1b), so the collection element needs no change for it.

use rune::PointerButton;

use super::{Autoscroll, AutoscrollSettings, Start, Target};
use crate::collection::element::{
    Ctx, Effect, Flow, KeyInput, PointerController, PointerInput, PointerPhase, viewport_path,
};
use crate::scroll::ScrollFeed;

type BackgroundFn = fn(&PointerInput, &Ctx<'_>, &AutoscrollSettings) -> bool;

/// The collection's pointer controller with middle-click autoscroll in front of it.
///
/// A middle press on the collection's background starts the window's [`Autoscroll`] when the
/// listing can scroll, and the controller inside never sees that press. Everything else goes
/// to the controller inside. Install it in the controller's place:
///
/// ```ignore
/// coll.controllers.pointer = Some(Box::new(AutoscrollPointer::new(
///     Some(Box::new(PointerIntent::new())), auto.clone(), "files", feed.clone())));
/// ```
///
/// `feed` is the collection's scroll position, which the host fills from
/// `Collection::on_scroll`. The host passes [`AutoscrollSettings`] with
/// `Collection::settings`, and mounts the [`Shield`](super::Shield).
///
/// Used by: C2 (one per column, all sharing the window's autoscroll), C3, C4, hosts.
pub struct AutoscrollPointer {
    inner: Option<Box<dyn PointerController>>,
    auto: Autoscroll,
    scope: String,
    feed: ScrollFeed,
    sideways: Option<Target>,
    background: BackgroundFn,
    /// The gesture in progress is one this decorator took (it started or stopped an
    /// autoscroll): its moves and release go no further.
    taken: bool,
}

impl AutoscrollPointer {
    /// Autoscroll in front of `inner` (C1b's, or `None` for the element's default), for the
    /// collection keyed `scope`, whose position `feed` holds, starting `auto`.
    pub fn new(
        inner: Option<Box<dyn PointerController>>,
        auto: Autoscroll,
        scope: impl Into<String>,
        feed: ScrollFeed,
    ) -> Self {
        Self {
            inner,
            auto,
            scope: scope.into(),
            feed,
            sideways: None,
            background: default_background,
            taken: false,
        }
    }

    /// Also scroll the sideways viewport `target` around the collection (C4's table, wider
    /// than its pane), on the horizontal axis.
    pub fn sideways(mut self, target: Target) -> Self {
        self.sideways = Some(target);
        self
    }

    /// What counts as the collection's background, where a middle press starts autoscroll:
    /// [`default_background`] unless a view's rows differ (C3's cards).
    pub fn background(mut self, f: BackgroundFn) -> Self {
        self.background = f;
        self
    }

    fn pass(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        match self.inner.as_mut() {
            Some(p) => p.pointer(input, cx),
            None => crate::collection::element::default_pointer(input, cx),
        }
    }
}

/// Strata's background (`ui::pointer::is_background`): no entry under the press, or the blank
/// strip of [`AutoscrollSettings::margin`] along either side of a row, which lies outside the
/// row in Strata's lists.
pub fn default_background(input: &PointerInput, cx: &Ctx<'_>, s: &AutoscrollSettings) -> bool {
    let x = input.point.x - cx.visible.x;
    input.hit.index.is_none() || x < s.margin || x >= cx.visible.w - s.margin
}

impl PointerController for AutoscrollPointer {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        let settings = cx
            .settings::<AutoscrollSettings>()
            .copied()
            .unwrap_or_default();
        if input.phase == PointerPhase::Down {
            self.taken = false;
            if self.auto.active() {
                // The shield takes every press while autoscroll runs; this is a host without
                // one. Stop dead where the views are on the next drive, which has the
                // transaction to stop every target's glide (another collection's, a sideways
                // scroll's), and keep the press from the controller.
                self.auto.request_stop();
                cx.emit(Effect::WakeAfter(std::time::Duration::ZERO));
                cx.repaint();
                self.taken = true;
                return Flow::Handled;
            }
            if input.button == PointerButton::Middle && (self.background)(input, cx, &settings) {
                let viewport = cx.content_to_window(cx.visible);
                let mut targets = vec![Target::vertical(
                    viewport_path(&self.scope),
                    self.feed.clone(),
                )];
                targets.extend(self.sideways.clone());
                let start = Start {
                    owner: self.scope.clone(),
                    anchor: input.window,
                    viewport,
                    targets,
                };
                if self.auto.start(start, settings) {
                    cx.repaint();
                    self.taken = true;
                    return Flow::Handled;
                }
                if input.hit.index.is_some() {
                    // Background beside or within an entry's row (its margin, a tile's blank
                    // space) in a listing that fits: Strata's start declines and the press
                    // reaches no row there, so it does nothing (no new tab).
                    self.taken = true;
                    return Flow::Handled;
                }
            }
            return self.pass(input, cx);
        }
        if self.taken {
            // A held middle drag: its moves steer, as the shield's would.
            if input.phase == PointerPhase::Move && self.auto.track(input.window) {
                cx.emit(Effect::WakeAfter(std::time::Duration::ZERO));
            }
            if input.phase == PointerPhase::Up {
                self.taken = false;
            }
            return Flow::Handled;
        }
        self.pass(input, cx)
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        match self.inner.as_mut() {
            Some(p) => p.tick(cx),
            None => Flow::Pass,
        }
    }

    fn key_seen(&mut self, input: &KeyInput<'_>) {
        if let Some(p) = self.inner.as_mut() {
            p.key_seen(input);
        }
    }

    fn reset(&mut self) {
        self.taken = false;
        if let Some(p) = self.inner.as_mut() {
            p.reset();
        }
        // Another folder: its autoscroll goes with it. `CollectionState::enter` scrolls
        // instantly, which stops the glide.
        self.auto.stop_gone(Some(&self.scope));
    }
}

impl std::fmt::Debug for AutoscrollPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoscrollPointer")
            .field("scope", &self.scope)
            .field("inner", &self.inner.is_some())
            .finish()
    }
}
