// SPDX-License-Identifier: MIT

//! The pointer sequences: moving, clicking, dragging and turning the wheel, with the gaps
//! the application needs between the events.

use std::time::{Duration, Instant};

use serde_json::json;

use super::{
    APPROACH_OFFSET, ARRIVAL_ATTEMPTS, BUTTON_HOLD, DOUBLE_CLICK_GAP, EVENT_GAP, LOCATE_TIMEOUT,
    POINTER_GAP, POLL_INTERVAL, REVEAL_SCROLL_ATTEMPTS, SELECT_RETRY_AFTER, SEPARATE_CLICK_GAP,
    locate::{Located, screen_rect},
    scope::Scope,
};
use crate::{
    Aim, At, Backend, DEFAULT_WAIT, EntryTarget, Failure, LookupError, Modifier, Orientation,
    Outcome, PressCheck, Rect, Target, coordinate, step_towards, travel, wheel_clicks,
};

pub(super) struct Aimed<'a> {
    pub(super) target: &'a Target,
    pub(super) at: Option<&'a At>,
    pub(super) left_bound: Option<&'a str>,
}

impl<'a> Aimed<'a> {
    pub(super) fn plain(target: &'a Target) -> Self {
        Self {
            target,
            at: None,
            left_bound: None,
        }
    }
}

pub(super) struct DragEnds<'a> {
    pub(super) from: &'a Target,
    pub(super) to: &'a Target,
    pub(super) from_at: Option<&'a At>,
    pub(super) to_at: Option<&'a At>,
}

/// The scroll pane that clips a point, the direction to scroll it in and how far.
struct Clipped<Node> {
    scroller: Node,
    viewport: Rect,
    along: Orientation,
    distance: f64,
}

impl<B: Backend> Scope<'_, B> {
    pub(super) async fn locate(
        &self,
        target: &Target,
        at: Option<&At>,
        aim: Aim,
    ) -> Result<(i16, i16), String> {
        let (x, y) = self.find(target, at, aim).await?.point;
        Ok((coordinate(x)?, coordinate(y)?))
    }

    /// A point a scroller clips is wheeled into view first, so the pointer lands on the
    /// target. A `left_bound` aim is never scrolled: it deliberately aims at the visible
    /// part of a clipped target.
    pub(super) async fn aim(&self, aim: &Aimed<'_>) -> Result<(i16, i16), String> {
        let mut scrolls = 0;
        loop {
            let located = self.find(aim.target, aim.at, Aim::Point).await?;
            let (x, y) = self.aimed_point(aim, &located)?;
            let may_scroll = aim.left_bound.is_none() && scrolls < REVEAL_SCROLL_ATTEMPTS;
            if may_scroll && self.scroll_onto_screen(&located, (x, y)).await? {
                scrolls += 1;
                continue;
            }
            return Ok((coordinate(x)?, coordinate(y)?));
        }
    }

    fn aimed_point(
        &self,
        aim: &Aimed<'_>,
        located: &Located<B::Node>,
    ) -> Result<(f64, f64), String> {
        let Some(name) = aim.left_bound else {
            return Ok(located.point);
        };
        let bound = self.kept.boxes.borrow().get(name).copied().ok_or_else(|| {
            format!("left_bound needs a box kept as {name:?} by remember or settle")
        })?;
        let left = located.rect.x.max(bound.x);
        Ok(Rect {
            x: left,
            width: located.rect.x + located.rect.width - left,
            ..located.rect
        }
        .point(aim.at))
    }

    fn clipping(
        &self,
        located: &Located<B::Node>,
        (x, y): (f64, f64),
    ) -> Result<Option<Clipped<B::Node>>, String> {
        let backend = self.backend;
        let rect = located.rect;
        let ancestors =
            std::iter::successors(backend.parent(&located.node), |above| backend.parent(above));
        for scroller in ancestors {
            if backend.scroll(&scroller, Orientation::Horizontal).is_none() {
                continue;
            }
            let viewport = match screen_rect(backend, &scroller) {
                Ok(viewport) => viewport,
                Err(LookupError::Missing(_)) => continue,
                Err(LookupError::Broken(error)) => return Err(error),
            };
            let axes = [
                (
                    Orientation::Horizontal,
                    (rect.x, rect.width),
                    x,
                    (viewport.x, viewport.width),
                ),
                (
                    Orientation::Vertical,
                    (rect.y, rect.height),
                    y,
                    (viewport.y, viewport.height),
                ),
            ];
            for (along, span, point, view) in axes {
                if let Some(distance) = travel(span, point, view)
                    && backend
                        .scroll(&scroller, along)
                        .is_some_and(|scroll| scroll.scrollable)
                {
                    return Ok(Some(Clipped {
                        scroller,
                        viewport,
                        along,
                        distance,
                    }));
                }
            }
        }
        Ok(None)
    }

    /// Turns the wheel over the scroller clipping `point` on `located`, with the wheel a user
    /// would turn: Shift for a horizontal strip such as Columns or the breadcrumbs. False when
    /// nothing clips it or the scroller would not move, so the caller aims where the target is.
    async fn scroll_onto_screen(
        &self,
        located: &Located<B::Node>,
        point: (f64, f64),
    ) -> Result<bool, String> {
        let backend = self.backend;
        let Some(clipped) = self.clipping(located, point)? else {
            return Ok(false);
        };
        let window =
            screen_rect(backend, &self.window).map_err(|error| error.message().to_owned())?;
        let left = clipped.viewport.x.max(window.x);
        let top = clipped.viewport.y.max(window.y);
        let right = (clipped.viewport.x + clipped.viewport.width).min(window.x + window.width);
        let bottom = (clipped.viewport.y + clipped.viewport.height).min(window.y + window.height);
        if right <= left || bottom <= top {
            return Ok(false);
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "screen coordinates are far inside i64"
        )]
        let at = (
            ((left + right) / 2.0).floor() as i64,
            ((top + bottom) / 2.0).floor() as i64,
        );
        let Some(before) = backend.scroll(&clipped.scroller, clipped.along) else {
            return Ok(false);
        };
        let modifiers: &[Modifier] = match clipped.along {
            Orientation::Horizontal => &[Modifier::Shift],
            Orientation::Vertical => &[],
        };
        self.wheel_at(
            at,
            wheel_clicks(clipped.distance, before.page),
            clipped.distance > 0.0,
            modifiers,
        )
        .await?;
        backend.settle(&self.window).await;
        let after = backend
            .scroll(&clipped.scroller, clipped.along)
            .map_or(before.value, |scroll| scroll.value);
        Ok((after - before.value).abs() > f64::EPSILON)
    }

    /// Clicks the middle of a node once it has come through a painted frame unmoved.
    pub(super) async fn click_node(
        &self,
        node: &B::Node,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let rect_of =
            || screen_rect(self.backend, node).map_err(|error| error.message().to_owned());
        let deadline = Instant::now() + LOCATE_TIMEOUT;
        let rect = loop {
            let before = rect_of()?;
            self.backend.settle(&self.window).await;
            let after = rect_of()?;
            if after.pixels() == before.pixels() {
                break after;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the row kept moving for {} s",
                    LOCATE_TIMEOUT.as_secs()
                ));
            }
        };
        let (x, y) = rect.centre();
        self.click_at((coordinate(x)?, coordinate(y)?), 1, 1, modifiers)
            .await
    }

    pub(super) async fn click(
        &self,
        aim: &Aimed<'_>,
        button: u8,
        count: u32,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let point = self.aim(aim).await?;
        self.click_at(point, button, count, modifiers).await
    }

    async fn click_at(
        &self,
        (x, y): (i16, i16),
        button: u8,
        count: u32,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let backend = self.backend;
        self.move_to(x, y).await?;
        backend.hold(&self.window, modifiers).await?;
        let mut result = Ok(());
        for click in 1..=count {
            result = self.tap_button(button, BUTTON_HOLD, click == count).await;
            if result.is_err() {
                break;
            }
            if click < count {
                backend.sleep(DOUBLE_CLICK_GAP).await;
            }
        }
        backend.release(&self.window, modifiers).await;
        self.pause(POINTER_GAP).await;
        result
    }

    /// Only the last release of a multi-click settles; a frame between the
    /// clicks could push them out of the double-click window.
    async fn tap_button(&self, number: u8, hold: Duration, last: bool) -> Result<(), String> {
        let backend = self.backend;
        backend.button(number, true).await?;
        backend.sleep(hold).await;
        backend.button(number, false).await?;
        self.kept.last_release.set(Some(Instant::now()));
        if last {
            self.pause(EVENT_GAP).await;
        } else {
            backend.sleep(EVENT_GAP).await;
        }
        Ok(())
    }

    pub(super) async fn click_releasing_modifiers_first(
        &self,
        aim: &Aimed<'_>,
        button: u8,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let backend = self.backend;
        let (x, y) = self.aim(aim).await?;
        self.move_to(x, y).await?;
        backend.hold(&self.window, modifiers).await?;
        let pressed = backend.button(button, true).await;
        let mut result = pressed;
        if result.is_ok() {
            backend.sleep(BUTTON_HOLD).await;
            backend.release(&self.window, modifiers).await;
            result = backend.button(button, false).await;
            self.kept.last_release.set(Some(Instant::now()));
            self.pause(EVENT_GAP).await;
        }
        backend.release(&self.window, modifiers).await;
        self.pause(POINTER_GAP).await;
        result
    }

    pub(super) async fn move_onto(&self, aim: &Aimed<'_>) -> Result<(), String> {
        let (x, y) = self.aim(aim).await?;
        self.move_to(x, y).await
    }

    pub(super) async fn move_by(&self, x: i16, y: i16) -> Result<(), String> {
        let (from_x, from_y) = self.backend.pointer_position()?;
        self.move_to(from_x.saturating_add(x), from_y.saturating_add(y))
            .await
    }

    pub(super) async fn wheel(
        &self,
        target: &Target,
        at: Option<&At>,
        clicks: u32,
        down: bool,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let point = self.locate(target, at, Aim::Point).await?;
        self.wheel_at(
            (i64::from(point.0), i64::from(point.1)),
            clicks,
            down,
            modifiers,
        )
        .await
    }

    /// Turns the wheel `clicks` times over a point on the screen.
    pub async fn wheel_at(
        &self,
        (x, y): (i64, i64),
        clicks: u32,
        down: bool,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let backend = self.backend;
        let x = i16::try_from(x).map_err(|error| format!("{x} is off the screen: {error}"))?;
        let y = i16::try_from(y).map_err(|error| format!("{y} is off the screen: {error}"))?;
        self.move_to(x, y).await?;
        backend.hold(&self.window, modifiers).await?;
        let mut result = Ok(());
        for _ in 0..clicks {
            result = self
                .tap_button(if down { 5 } else { 4 }, EVENT_GAP, true)
                .await;
            if result.is_err() {
                break;
            }
        }
        backend.release(&self.window, modifiers).await;
        result
    }

    /// Crosses in even steps so GTK passes its drag threshold and the drop site sees the hover.
    pub(super) async fn drag(
        &self,
        ends: &DragEnds<'_>,
        steps: u32,
        release: bool,
        modifiers: &[Modifier],
        after_press: &[PressCheck],
    ) -> Result<Option<Failure>, String> {
        let (start, end) = if ends.from_at.is_some() {
            let start = self.locate(ends.from, ends.from_at, Aim::Point).await?;
            (start, self.locate(ends.to, ends.to_at, Aim::Point).await?)
        } else {
            // Read both boxes only after both lookups, so a target a listing refresh moved
            // is aimed where it is now.
            self.find(ends.from, None, Aim::DragOrigin).await?;
            let end = self.locate(ends.to, ends.to_at, Aim::Point).await?;
            (self.locate(ends.from, None, Aim::DragOrigin).await?, end)
        };
        self.backend.hold(&self.window, modifiers).await?;
        let result = self
            .drag_between(start, end, steps, release, after_press)
            .await;
        self.backend.release(&self.window, modifiers).await;
        result
    }

    /// The checks a drag makes while its button is down, each given its own time to hold.
    async fn after_press(&self, checks: &[PressCheck]) -> Result<Option<Failure>, String> {
        for press in checks {
            let deadline = press.wait.map(|wait| Instant::now() + wait);
            loop {
                match self.check(&press.predicate)? {
                    Ok(()) => break,
                    Err(mut failure) => {
                        if deadline.is_some_and(|deadline| Instant::now() < deadline) {
                            self.backend.sleep(POLL_INTERVAL).await;
                            continue;
                        }
                        failure.message.clone_from(&press.message);
                        return Ok(Some(failure));
                    }
                }
            }
        }
        Ok(None)
    }

    async fn drag_between(
        &self,
        start: (i16, i16),
        end: (i16, i16),
        steps: u32,
        release: bool,
        after_press: &[PressCheck],
    ) -> Result<Option<Failure>, String> {
        let backend = self.backend;
        self.move_to(start.0, start.1).await?;
        if let Some(remaining) = self
            .kept
            .last_release
            .get()
            .and_then(|released| SEPARATE_CLICK_GAP.checked_sub(released.elapsed()))
        {
            backend.sleep(remaining).await;
        }
        self.press_button(1, true, POINTER_GAP).await?;
        let checked = self.after_press(after_press).await;
        if !matches!(checked, Ok(None)) {
            backend.button(1, false).await?;
            return checked;
        }
        let travel = async {
            for step in 1..=steps {
                let x = step_towards(start.0, end.0, step, steps);
                let y = step_towards(start.1, end.1, step, steps);
                self.motion(x, y, EVENT_GAP).await?;
            }
            self.motion(end.0, end.1, POINTER_GAP).await
        };
        let travelled = travel.await;
        if release || travelled.is_err() {
            self.press_button(1, false, POINTER_GAP).await?;
        }
        travelled.map(|()| None)
    }

    /// One button event, then a painted frame and what is left of `gap`.
    pub(super) async fn press_button(
        &self,
        number: u8,
        pressed: bool,
        gap: Duration,
    ) -> Result<(), String> {
        self.backend.button(number, pressed).await?;
        self.pause(gap).await;
        Ok(())
    }

    async fn motion(&self, x: i16, y: i16, gap: Duration) -> Result<(), String> {
        self.backend.pointer_move(x, y).await?;
        self.pause(gap).await;
        Ok(())
    }

    pub(super) async fn select(&self, target: &EntryTarget) -> Outcome {
        let wanted = Target::Entry(target.clone());
        let selected = || {
            self.scene()
                .observation
                .entry_selected(&target.entry, target.dir.as_deref())
        };
        for patience in [SELECT_RETRY_AFTER, DEFAULT_WAIT] {
            if let Err(error) = self.click(&Aimed::plain(&wanted), 1, 1, &[]).await {
                return Outcome::error(error);
            }
            let deadline = Instant::now() + patience;
            while Instant::now() < deadline {
                if selected() {
                    return Outcome::default();
                }
                self.backend.sleep(POLL_INTERVAL).await;
            }
        }
        Outcome {
            result: json!({ "tree": self.backend.dump(&self.window) }),
            failures: vec![Failure {
                predicate: json!({ "select": target }),
                actual: json!(false),
                message: Some(format!("{:?} to become selected", target.entry)),
            }],
            error: Some("the entry never became selected".to_owned()),
        }
    }

    /// Confirms the pointer arrived, since a press elsewhere lands on the wrong node.
    async fn move_to(&self, x: i16, y: i16) -> Result<(), String> {
        self.motion(
            x.saturating_sub(APPROACH_OFFSET),
            y.saturating_sub(APPROACH_OFFSET),
            EVENT_GAP,
        )
        .await?;
        self.motion(x, y, POINTER_GAP).await?;
        for _ in 0..ARRIVAL_ATTEMPTS {
            if self.backend.pointer_position()? == (x, y) {
                return Ok(());
            }
            self.motion(x, y, POINTER_GAP).await?;
        }
        Err(format!(
            "the pointer did not reach ({x}, {y}); it is at {:?}",
            self.backend.pointer_position()?
        ))
    }
}
