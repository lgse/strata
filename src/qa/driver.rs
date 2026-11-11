// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};

use super::{
    LOCATE_TIMEOUT, POLL_INTERVAL, REVEAL_SCROLL_ATTEMPTS,
    backend::Gtk,
    input::{Event, Injector},
    pointer::Aimed,
    protocol::{self, At, Scope, Target, coordinate},
    settle,
    targets::{self, Aim, Rect},
};

/// A step's lookups, with the input connection that GTK's own sequences send through.
pub(super) struct Input<'a> {
    pub(super) scope: Scope<'a, Gtk>,
    pub(super) injector: &'a Injector,
    pub(super) window: &'a gtk::Window,
}

impl<'a> std::ops::Deref for Input<'a> {
    type Target = Scope<'a, Gtk>;

    fn deref(&self) -> &Self::Target {
        &self.scope
    }
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
                if let Some(widget) =
                    protocol::find_presented(self.backend, &self.scope.window, role, name)
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
                .node
        };
        if !widget.activate() {
            return Err(format!("{} has no default action", target.to_json()));
        }
        settle(Some(self.window)).await;
        Ok(())
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
        located: &targets::Located,
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
}
