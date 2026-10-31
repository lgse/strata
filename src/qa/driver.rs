// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use super::{
    LOCATE_TIMEOUT, POLL_INTERVAL,
    input::{Event, Injector},
    pointer::Aimed,
    protocol::{At, FieldRef, Target},
    settle, state,
    targets::{self, Aim, Boxes, Kept, LookupError, Memory, Rect},
};

pub(super) struct Input<'a> {
    pub(super) injector: &'a Injector,
    pub(super) window: &'a gtk::Window,
    pub(super) memory: &'a RefCell<Memory>,
    pub(super) widgets: &'a RefCell<Memory>,
    pub(super) boxes: &'a RefCell<Boxes>,
}

impl Input<'_> {
    pub(super) async fn pause(&self, gap: Duration) {
        let started = Instant::now();
        settle(Some(self.window)).await;
        if let Some(remaining) = gap.checked_sub(started.elapsed()) {
            glib::timeout_future(remaining).await;
        }
    }

    pub(super) async fn emit(&self, event: Event, gap: Duration) -> Result<(), String> {
        self.injector.send(event)?;
        self.pause(gap).await;
        Ok(())
    }

    pub(super) fn locate_now(
        &self,
        target: &Target,
        at: Option<&At>,
        aim: Aim,
    ) -> Result<targets::Located, LookupError> {
        targets::locate(
            self.window,
            self.injector,
            Kept {
                memory: &self.memory.borrow(),
                widgets: &self.widgets.borrow(),
                boxes: &self.boxes.borrow(),
            },
            target,
            at,
            aim,
        )
    }

    pub(super) fn presented(&self, target: &Target) -> Result<Option<gtk::Widget>, String> {
        targets::presented_target(
            self.window,
            self.injector,
            Kept {
                memory: &self.memory.borrow(),
                widgets: &self.widgets.borrow(),
                boxes: &self.boxes.borrow(),
            },
            target,
        )
        .map_err(|error| error.message().to_owned())
    }

    pub(super) fn resolve(&self, target: &Target) -> Result<Option<gtk::Widget>, String> {
        if matches!(
            target,
            Target::Role {
                rendered: false,
                ..
            }
        ) {
            return self.presented(target);
        }
        match self.locate_now(target, None, Aim::Point) {
            Ok(located) => Ok(Some(located.widget)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(LookupError::Broken(error)) => Err(error),
        }
    }

    pub(super) fn field(&self, field: &FieldRef) -> Result<Option<gtk::Widget>, String> {
        targets::field(self.window, &self.memory.borrow(), field).map_err(|error| match error {
            LookupError::Missing(message) | LookupError::Broken(message) => message,
        })
    }

    pub(super) async fn activate(
        &self,
        target: &Target,
        message: Option<&str>,
    ) -> Result<(), String> {
        let widget = if let Target::Role {
            rendered: false, ..
        } = target
        {
            let deadline = Instant::now() + LOCATE_TIMEOUT;
            loop {
                if let Some(widget) = self.presented(target)? {
                    break widget;
                }
                if Instant::now() >= deadline {
                    let waiting = message.map_or_else(String::new, |message| {
                        format!("timed out waiting for {message}: ")
                    });
                    return Err(format!(
                        "{waiting}no {} presented after {} s",
                        target.to_json(),
                        LOCATE_TIMEOUT.as_secs()
                    ));
                }
                glib::timeout_future(POLL_INTERVAL).await;
            }
        } else if let Target::Role {
            role,
            other_roles,
            name: Some(name),
            description: None,
            within: None,
            name_ends_with,
            nth: None,
            states,
            name_matches: None,
            has: None,
            rendered: true,
        } = target
            && name_ends_with.is_empty()
            && states.is_empty()
            && other_roles.is_empty()
        {
            let deadline = Instant::now() + LOCATE_TIMEOUT;
            loop {
                if let Some(widget) = targets::find_presented(self.window.upcast_ref(), role, name)
                {
                    break widget;
                }
                if Instant::now() >= deadline {
                    let waiting = message.map_or_else(String::new, |message| {
                        format!("timed out waiting for {message}: ")
                    });
                    return Err(format!(
                        "{waiting}no {role} named {name:?} after {} s",
                        LOCATE_TIMEOUT.as_secs()
                    ));
                }
                glib::timeout_future(POLL_INTERVAL).await;
            }
        } else {
            self.find(target, None, Aim::Point)
                .await
                .map_err(|error| match message {
                    Some(message) => format!("timed out waiting for {message}: {error}"),
                    None => error,
                })?
                .widget
        };
        if !widget.activate() {
            return Err(format!("{} has no default action", target.to_json()));
        }
        settle(Some(self.window)).await;
        Ok(())
    }

    /// A row a listing just added can report a box before its first allocation lands, so the
    /// box must come through a painted frame unchanged.
    pub(super) async fn find(
        &self,
        target: &Target,
        at: Option<&At>,
        aim: Aim,
    ) -> Result<targets::Located, String> {
        let deadline = Instant::now() + LOCATE_TIMEOUT;
        loop {
            match self.locate_now(target, at, aim) {
                Ok(located) => {
                    settle(Some(self.window)).await;
                    match self.locate_now(target, at, aim) {
                        Ok(painted) if painted.rect.pixels() == located.rect.pixels() => {
                            return Ok(painted);
                        }
                        Err(LookupError::Broken(error)) => return Err(error),
                        _ if Instant::now() >= deadline => {
                            return Err(format!(
                                "the target kept moving for {} s",
                                LOCATE_TIMEOUT.as_secs()
                            ));
                        }
                        _ => {}
                    }
                }
                Err(LookupError::Broken(error)) => return Err(error),
                Err(LookupError::Missing(error)) if Instant::now() >= deadline => {
                    return Err(format!("{error} after {} s", LOCATE_TIMEOUT.as_secs()));
                }
                Err(LookupError::Missing(_)) => glib::timeout_future(POLL_INTERVAL).await,
            }
        }
    }

    pub(super) async fn locate(
        &self,
        target: &Target,
        at: Option<&At>,
        aim: Aim,
    ) -> Result<(i16, i16), String> {
        let (x, y) = self.find(target, at, aim).await?.point;
        Ok((coordinate(x)?, coordinate(y)?))
    }

    pub(super) async fn aim(&self, aim: &Aimed<'_>) -> Result<(i16, i16), String> {
        let located = self.find(aim.target, aim.at, Aim::Point).await?;
        let (x, y) = match aim.left_bound {
            None => located.point,
            Some(name) => {
                let bound = self.boxes.borrow().get(name).copied().ok_or_else(|| {
                    format!("left_bound needs a box kept as {name:?} by remember or settle")
                })?;
                let left = located.rect.x.max(bound.x);
                Rect {
                    x: left,
                    width: located.rect.x + located.rect.width - left,
                    ..located.rect
                }
                .point(aim.at)
            }
        };
        Ok((coordinate(x)?, coordinate(y)?))
    }

    pub(super) fn focused_widget(&self) -> Option<gtk::Widget> {
        targets::presented(self.window.upcast_ref())
            .into_iter()
            .find(|widget| {
                targets::states(widget).focused && state::rendered_bounds(widget).is_some()
            })
    }
}

pub(super) fn coordinate(value: f64) -> Result<i16, String> {
    let value = value.floor();
    if value < f64::from(i16::MIN) || value > f64::from(i16::MAX) {
        return Err(format!("the target lies outside the screen at {value}"));
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the range was checked just above"
    )]
    Ok(value as i16)
}
