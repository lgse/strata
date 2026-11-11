// SPDX-License-Identifier: MIT

use std::time::Instant;

use gtk::{glib, prelude::*};
use serde_json::json;

use super::{
    Outcome, POLL_INTERVAL, REVEAL_SCROLL_ATTEMPTS, WHEEL_REVEAL_CLICKS,
    backend::Gtk,
    protocol::{self, Backend as _, Check, Failure, Orientation, Predicate, Scope, Target},
    state,
    targets::{self, LookupError},
};

impl Gtk {
    async fn presented_control(
        &self,
        scope: &Scope<'_, Self>,
        target: &Target,
        message: String,
    ) -> Result<(gtk::Widget, gtk::Widget), Outcome> {
        let injector = self.injector().map_err(Outcome::error)?;
        let wanted = protocol::Wanted::of(target)
            .map_err(|error| Outcome::error(error.message().to_owned()))?;
        let deadline = Instant::now() + protocol::DEFAULT_WAIT;
        let node = loop {
            if let Some(node) = wanted.find_presented(self, &scope.window) {
                break node;
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "presented": target }),
                    actual: json!(false),
                    message: Some(message),
                };
                return Err(scope.timed_out(failure, protocol::DEFAULT_WAIT));
            }
            glib::timeout_future(POLL_INTERVAL).await;
        };
        let scroll = std::iter::successors(node.parent(), WidgetExt::parent)
            .find(|widget| targets::role_name(widget) == "scroll pane")
            .ok_or_else(|| Outcome::error("the control has no scroll pane around it".to_owned()))?;
        // A page just shown can report a box before its layout lands, which the
        // slower AT-SPI reads never see; aim only at one a painted frame keeps.
        loop {
            let before = targets::extents(&node, injector).map(|rect| rect.pixels());
            self.settle(&scope.window).await;
            let after = targets::extents(&node, injector).map(|rect| rect.pixels());
            match (before, after) {
                (Ok(before), Ok(after)) if before == after && after[2] > 0 && after[3] > 0 => break,
                (_, Err(LookupError::Broken(error))) => return Err(Outcome::error(error)),
                _ if Instant::now() >= deadline => {
                    return Err(Outcome::error(format!(
                        "{message}: the control never came to rest"
                    )));
                }
                _ => glib::timeout_future(POLL_INTERVAL).await,
            }
        }
        Ok((node, scroll))
    }

    pub(super) async fn reveal_control(&self, scope: &Scope<'_, Self>, target: &Target) -> Outcome {
        let injector = match self.injector() {
            Ok(injector) => injector,
            Err(error) => return Outcome::error(error),
        };
        let name = control_name(target);
        let (node, scroll) = match self
            .presented_control(scope, target, format!("{name} control"))
            .await
        {
            Ok(found) => found,
            Err(outcome) => return outcome,
        };
        let deadline = Instant::now() + protocol::DEFAULT_WAIT;
        loop {
            let (bounds, viewport) = match (
                targets::extents(&node, injector),
                targets::extents(&scroll, injector),
            ) {
                (Ok(bounds), Ok(viewport)) => (bounds.pixels(), viewport.pixels()),
                (Err(LookupError::Broken(error)), _) | (_, Err(LookupError::Broken(error))) => {
                    return Outcome::error(error);
                }
                _ => {
                    if Instant::now() >= deadline {
                        return Outcome::error(format!("{name} has no bounds"));
                    }
                    glib::timeout_future(POLL_INTERVAL).await;
                    continue;
                }
            };
            if state::inside(bounds, viewport, None) {
                break;
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "reveal_control": target }),
                    actual: json!({ "bounds": bounds, "viewport": viewport }),
                    message: Some(format!("reachable {name} control")),
                };
                return scope.timed_out(failure, protocol::DEFAULT_WAIT);
            }
            let [view_x, view_y, view_width, view_height] = viewport;
            let at = (view_x + view_width - 16, view_y + view_height.div_euclid(2));
            let below = bounds[1] + bounds[3] - (view_y + view_height);
            let above = view_y - bounds[1];
            let distance = below.max(above);
            #[expect(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                reason = "Python's int() of a true division of pixel counts"
            )]
            let steps = (distance as f64 / view_height as f64).trunc() as i64 * 3;
            let clicks = steps.clamp(1, 12);
            #[expect(clippy::cast_sign_loss, reason = "clamped to 1..=12")]
            let wheeled = scope.wheel_at(at, clicks as u32, below > 0, &[]).await;
            if let Err(error) = wheeled {
                return Outcome::error(error);
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
        scope
            .settle_target(&presented(target), protocol::DEFAULT_WAIT, None)
            .await
    }

    /// `Strata.reveal`, whose `scroll_into_view` GTK's bridge declines.
    pub(super) async fn reveal(&self, scope: &Scope<'_, Self>, target: &Target) -> Outcome {
        let injector = match self.injector() {
            Ok(injector) => injector,
            Err(error) => return Outcome::error(error),
        };
        let criteria = match target {
            Target::Role {
                role,
                name: Some(name),
                ..
            } => format!(
                "{{'role': {}, 'name': {}}}",
                python_repr(role),
                python_repr(name)
            ),
            other => other.to_json().to_string(),
        };
        let (node, scroll) = match self
            .presented_control(scope, target, format!("a control matching {criteria}"))
            .await
        {
            Ok(found) => found,
            Err(outcome) => return outcome,
        };
        let visible = || -> Result<bool, String> {
            let rect =
                |widget: &gtk::Widget| targets::extents(widget, injector).map(|rect| rect.pixels());
            match (rect(&node), rect(&scroll), rect(&scope.window)) {
                (Ok(bounds), Ok(pane), Ok(window)) => Ok(state::on_screen(bounds, window)
                    && bounds[1] >= pane[1]
                    && bounds[1] + bounds[3] <= pane[1] + pane[3]),
                (Err(LookupError::Broken(error)), _, _)
                | (_, Err(LookupError::Broken(error)), _)
                | (_, _, Err(LookupError::Broken(error))) => Err(error),
                _ => Ok(false),
            }
        };
        match visible() {
            Err(error) => return Outcome::error(error),
            Ok(true) => return Outcome::default(),
            Ok(false) => {}
        }
        let window = match protocol::screen_rect(self, &scope.window) {
            Ok(window) => window.pixels(),
            Err(error) => return Outcome::error(error.message().to_owned()),
        };
        let centre = (
            window[0] + window[2].div_euclid(2),
            window[1] + window[3].div_euclid(2),
        );
        for _ in 0..REVEAL_SCROLL_ATTEMPTS {
            match visible() {
                Err(error) => return Outcome::error(error),
                Ok(true) => {
                    return scope
                        .settle_target(&presented(target), protocol::DEFAULT_WAIT, None)
                        .await;
                }
                Ok(false) => {}
            }
            let below = match targets::extents(&node, injector) {
                Ok(rect) => rect.pixels()[1] > centre.1,
                Err(error) => return Outcome::error(error.message().to_owned()),
            };
            if let Err(error) = scope
                .wheel_at(centre, WHEEL_REVEAL_CLICKS, below, &[])
                .await
            {
                return Outcome::error(error);
            }
        }
        let on_screen = Predicate {
            check: Check::OnScreen(presented(target)),
            dir: None,
        };
        let deadline = Instant::now() + protocol::DEFAULT_WAIT;
        loop {
            match scope.check(&on_screen) {
                Err(error) => return Outcome::error(error),
                Ok(Ok(())) => return Outcome::default(),
                Ok(Err(mut failure)) if Instant::now() >= deadline => {
                    failure.message = Some(format!(
                        "<{} {}> to be scrolled into view",
                        targets::role_name(&node),
                        python_repr(&targets::accessible_name(&node))
                    ));
                    return scope.timed_out(failure, protocol::DEFAULT_WAIT);
                }
                Ok(Err(_)) => glib::timeout_future(POLL_INTERVAL).await,
            }
        }
    }
}

fn presented(target: &Target) -> Target {
    let mut target = target.clone();
    if let Target::Role { rendered, .. } = &mut target {
        *rendered = false;
    }
    target
}

fn control_name(target: &Target) -> String {
    match target {
        Target::Role {
            name: Some(name), ..
        } => name.clone(),
        other => other.to_json().to_string(),
    }
}

fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut printed = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => printed.push_str("\\\\"),
            '\n' => printed.push_str("\\n"),
            '\t' => printed.push_str("\\t"),
            character if character == quote => {
                printed.push('\\');
                printed.push(character);
            }
            character => printed.push(character),
        }
    }
    printed.push(quote);
    printed
}

pub(super) fn set_scrollbar(
    window: &gtk::Widget,
    orientation: Orientation,
    value: f64,
) -> Result<(), String> {
    let wanted = match orientation {
        Orientation::Horizontal => gtk::Orientation::Horizontal,
        Orientation::Vertical => gtk::Orientation::Vertical,
    };
    let scrollbar = targets::presented(window)
        .into_iter()
        .find_map(|widget| {
            widget
                .downcast::<gtk::Scrollbar>()
                .ok()
                .filter(|scrollbar| scrollbar.orientation() == wanted)
        })
        .ok_or_else(|| format!("no {orientation:?} scroll bar"))?;
    let adjustment = scrollbar.adjustment();
    adjustment.set_value(value);
    if (adjustment.value() - value).abs() > f64::EPSILON {
        return Err(format!(
            "the {orientation:?} scroll bar took {} instead of {value}",
            adjustment.value()
        ));
    }
    Ok(())
}
