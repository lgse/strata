// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::glib;
use serde_json::json;

use super::{
    APPROACH_OFFSET, ARRIVAL_ATTEMPTS, BUTTON_HOLD, DOUBLE_CLICK_GAP, EVENT_GAP, LOCATE_TIMEOUT,
    Outcome, POINTER_GAP, POLL_INTERVAL, SELECT_RETRY_AFTER, SEPARATE_CLICK_GAP,
    driver::{Input, coordinate},
    input::Event,
    protocol::{self, At, EntryTarget, Failure, Modifier, Predicate, PressCheck, Target},
    settle, state,
    targets::{self, Aim},
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

type Verdict = Result<Result<(), Failure>, String>;

pub(super) struct AfterPress<'a> {
    pub(super) checks: &'a [PressCheck],
    pub(super) check: &'a dyn Fn(&Predicate) -> Verdict,
}

impl AfterPress<'_> {
    async fn run(&self) -> Result<Option<Failure>, String> {
        for press in self.checks {
            let deadline = press.wait.map(|wait| Instant::now() + wait);
            loop {
                match (self.check)(&press.predicate)? {
                    Ok(()) => break,
                    Err(mut failure) => {
                        if deadline.is_some_and(|deadline| Instant::now() < deadline) {
                            glib::timeout_future(POLL_INTERVAL).await;
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
}

impl Input<'_> {
    pub(super) async fn click_widget(
        &self,
        widget: &gtk::Widget,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let deadline = Instant::now() + LOCATE_TIMEOUT;
        let rect = loop {
            let before = targets::screen_rect(widget, self.injector)
                .map_err(|error| error.message().to_owned())?;
            settle(Some(self.window)).await;
            let after = targets::screen_rect(widget, self.injector)
                .map_err(|error| error.message().to_owned())?;
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
        self.move_to(x, y).await?;
        let held = self.hold(modifiers).await?;
        let mut result = Ok(());
        for click in 1..=count {
            result = self.tap_button(button, BUTTON_HOLD, click == count).await;
            if result.is_err() {
                break;
            }
            if click < count {
                glib::timeout_future(DOUBLE_CLICK_GAP).await;
            }
        }
        self.release(&held).await;
        self.pause(POINTER_GAP).await;
        result
    }

    /// Only the last release of a multi-click settles; a frame between the
    /// clicks could push them out of the double-click window.
    async fn tap_button(&self, number: u8, hold: Duration, last: bool) -> Result<(), String> {
        self.injector.send(Event::Button {
            number,
            pressed: true,
        })?;
        glib::timeout_future(hold).await;
        self.injector.send(Event::Button {
            number,
            pressed: false,
        })?;
        self.injector.record_release();
        if last {
            self.pause(EVENT_GAP).await;
        } else {
            glib::timeout_future(EVENT_GAP).await;
        }
        Ok(())
    }

    pub(super) async fn click_releasing_modifiers_first(
        &self,
        aim: &Aimed<'_>,
        button: u8,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let (x, y) = self.aim(aim).await?;
        self.move_to(x, y).await?;
        let held = self.hold(modifiers).await?;
        let pressed = self.injector.send(Event::Button {
            number: button,
            pressed: true,
        });
        let mut result = pressed;
        if result.is_ok() {
            glib::timeout_future(BUTTON_HOLD).await;
            self.release(&held).await;
            result = self.injector.send(Event::Button {
                number: button,
                pressed: false,
            });
            self.injector.record_release();
            self.pause(EVENT_GAP).await;
        }
        self.release(&held).await;
        self.pause(POINTER_GAP).await;
        result
    }

    pub(super) async fn move_onto(&self, aim: &Aimed<'_>) -> Result<(), String> {
        let (x, y) = self.aim(aim).await?;
        self.move_to(x, y).await
    }

    pub(super) async fn move_by(&self, x: i16, y: i16) -> Result<(), String> {
        let (from_x, from_y) = self.injector.pointer_position()?;
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

    pub(super) async fn wheel_at(
        &self,
        (x, y): (i64, i64),
        clicks: u32,
        down: bool,
        modifiers: &[Modifier],
    ) -> Result<(), String> {
        let x = i16::try_from(x).map_err(|error| format!("{x} is off the screen: {error}"))?;
        let y = i16::try_from(y).map_err(|error| format!("{y} is off the screen: {error}"))?;
        self.move_to(x, y).await?;
        let held = self.hold(modifiers).await?;
        let mut result = Ok(());
        for _ in 0..clicks {
            result = self
                .tap_button(if down { 5 } else { 4 }, EVENT_GAP, true)
                .await;
            if result.is_err() {
                break;
            }
        }
        self.release(&held).await;
        result
    }

    /// Crosses in even steps so GTK passes its drag threshold and the drop site sees the hover.
    pub(super) async fn drag(
        &self,
        ends: &DragEnds<'_>,
        steps: u32,
        release: bool,
        modifiers: &[Modifier],
        after_press: &AfterPress<'_>,
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
        let held = self.hold(modifiers).await?;
        let result = self
            .drag_between(start, end, steps, release, after_press)
            .await;
        self.release(&held).await;
        result
    }

    async fn drag_between(
        &self,
        start: (i16, i16),
        end: (i16, i16),
        steps: u32,
        release: bool,
        after_press: &AfterPress<'_>,
    ) -> Result<Option<Failure>, String> {
        self.move_to(start.0, start.1).await?;
        if let Some(remaining) = self
            .injector
            .last_release()
            .and_then(|released| SEPARATE_CLICK_GAP.checked_sub(released.elapsed()))
        {
            glib::timeout_future(remaining).await;
        }
        self.emit(
            Event::Button {
                number: 1,
                pressed: true,
            },
            POINTER_GAP,
        )
        .await?;
        let checked = after_press.run().await;
        if !matches!(checked, Ok(None)) {
            self.injector.send(Event::Button {
                number: 1,
                pressed: false,
            })?;
            return checked;
        }
        let travel = async {
            for step in 1..=steps {
                let x = step_towards(start.0, end.0, step, steps);
                let y = step_towards(start.1, end.1, step, steps);
                self.emit(Event::Motion { x, y }, EVENT_GAP).await?;
            }
            self.emit(Event::Motion { x: end.0, y: end.1 }, POINTER_GAP)
                .await
        };
        let travelled = travel.await;
        if release || travelled.is_err() {
            self.emit(
                Event::Button {
                    number: 1,
                    pressed: false,
                },
                POINTER_GAP,
            )
            .await?;
        }
        travelled.map(|()| None)
    }

    pub(super) async fn select(&self, target: &EntryTarget) -> Outcome {
        let wanted = Target::Entry(target.clone());
        let selected = || {
            state::scan(self.window)
                .observation
                .entry_selected(&target.entry, target.dir.as_deref())
        };
        for patience in [SELECT_RETRY_AFTER, protocol::DEFAULT_WAIT] {
            if let Err(error) = self.click(&Aimed::plain(&wanted), 1, 1, &[]).await {
                return Outcome::error(error);
            }
            let deadline = Instant::now() + patience;
            while Instant::now() < deadline {
                if selected() {
                    return Outcome::default();
                }
                glib::timeout_future(POLL_INTERVAL).await;
            }
        }
        Outcome {
            result: json!({ "tree": state::dump(self.window) }),
            failures: vec![Failure {
                predicate: json!({ "select": target }),
                actual: json!(false),
                message: Some(format!("{:?} to become selected", target.entry)),
            }],
            error: Some("the entry never became selected".to_owned()),
        }
    }

    /// Confirms the server moved the pointer, since a press elsewhere lands on the wrong widget.
    /// On Wayland the position is the injector's own record, so a lost motion goes unnoticed.
    async fn move_to(&self, x: i16, y: i16) -> Result<(), String> {
        self.emit(
            Event::Motion {
                x: x.saturating_sub(APPROACH_OFFSET),
                y: y.saturating_sub(APPROACH_OFFSET),
            },
            EVENT_GAP,
        )
        .await?;
        self.emit(Event::Motion { x, y }, POINTER_GAP).await?;
        for _ in 0..ARRIVAL_ATTEMPTS {
            if self.injector.pointer_position()? == (x, y) {
                return Ok(());
            }
            self.emit(Event::Motion { x, y }, POINTER_GAP).await?;
        }
        Err(format!(
            "the pointer did not reach ({x}, {y}); it is at {:?}",
            self.injector.pointer_position()?
        ))
    }
}

/// Python's floor division, so both harnesses visit the same pixels.
pub(super) fn step_towards(from: i16, to: i16, step: u32, steps: u32) -> i16 {
    let travelled =
        ((i64::from(to) - i64::from(from)) * i64::from(step)).div_euclid(i64::from(steps));
    i16::try_from(i64::from(from) + travelled).unwrap_or(to)
}
