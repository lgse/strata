// SPDX-License-Identifier: MIT

use rune::PointerButton;

use super::{Autoscroll, AutoscrollSettings, Start, Target};
use crate::collection::element::{
    Ctx, Effect, Flow, KeyInput, PointerController, PointerInput, PointerPhase, viewport_path,
};
use crate::scroll::ScrollFeed;

type BackgroundFn = fn(&PointerInput, &Ctx<'_>, &AutoscrollSettings) -> bool;

pub struct AutoscrollPointer {
    inner: Option<Box<dyn PointerController>>,
    auto: Autoscroll,
    scope: String,
    feed: ScrollFeed,
    sideways: Option<Target>,
    background: BackgroundFn,
    taken: bool,
}

impl AutoscrollPointer {
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

    pub fn sideways(mut self, target: Target) -> Self {
        self.sideways = Some(target);
        self
    }

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

    fn needs_tick(&self) -> bool {
        self.inner.as_ref().is_some_and(|p| p.needs_tick())
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
