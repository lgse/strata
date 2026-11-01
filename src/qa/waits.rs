// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::glib;
use serde_json::json;

use super::{
    Controller, MENU_RETRY_AFTER, Outcome, POLL_INTERVAL, SETTLE_SAMPLES,
    driver::Input,
    pointer::Aimed,
    protocol::{self, At, Failure, Modifier, Predicate, Repeat, StateName, Target},
    settle,
    targets::{self, Aim, LookupError},
};

impl Controller {
    pub(super) async fn adopt(
        &self,
        title: &str,
        timeout: Duration,
        message: Option<&str>,
    ) -> Outcome {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(window) = targets::window_titled(title, None) {
                self.main.set(Some(&window));
                settle(Some(&window)).await;
                return Outcome::default();
            }
            if Instant::now() >= deadline {
                return Outcome {
                    failures: vec![Failure {
                        predicate: json!({ "adopt": title }),
                        actual: json!(false),
                        message: message.map(str::to_owned),
                    }],
                    error: Some(format!("timed out after {} ms", timeout.as_millis())),
                    ..Outcome::default()
                };
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
    }

    /// A late startup focus change can close the menu as it opens, so it is re-clicked each second.
    pub(super) async fn open_menu(
        &self,
        input: &Input<'_>,
        target: &Target,
        until: &Predicate,
        message: Option<String>,
        open_states: &[StateName],
    ) -> Outcome {
        let aim = Aimed::plain(target);
        if let Err(error) = input.click(&aim, 1, 1, &[]).await {
            return Outcome::error(error);
        }
        let mut clicked = Instant::now();
        let deadline = clicked + protocol::DEFAULT_WAIT;
        loop {
            match self.check(input, until) {
                Err(error) => return Outcome::error(error),
                Ok(Ok(())) => return Outcome::default(),
                Ok(Err(mut failure)) if Instant::now() >= deadline => {
                    failure.message = message;
                    return Outcome::timed_out(input.window, failure, protocol::DEFAULT_WAIT);
                }
                Ok(Err(_)) => {}
            }
            // GTK never reports `checked` for a toggle button, so re-click while the menu is shut.
            let open = input
                .locate_now(target, None, Aim::Point)
                .is_ok_and(|located| {
                    open_states
                        .iter()
                        .any(|state| targets::has_state(&located.widget, *state))
                });
            if clicked.elapsed() > MENU_RETRY_AFTER && !open {
                if let Err(error) = input.click(&aim, 1, 1, &[]).await {
                    return Outcome::error(error);
                }
                clicked = Instant::now();
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
    }

    pub(super) async fn settle_target(
        &self,
        input: &Input<'_>,
        target: &Target,
        timeout: Duration,
        name: Option<String>,
    ) -> Outcome {
        let deadline = Instant::now() + timeout;
        let mut history: Vec<Option<[i64; 4]>> = Vec::new();
        loop {
            let located = input.locate_now(target, None, Aim::Point);
            if let Err(LookupError::Broken(error)) = located {
                return Outcome::error(error);
            }
            history.push(located.as_ref().ok().map(|located| located.rect.pixels()));
            if history.len() > SETTLE_SAMPLES {
                history.remove(0);
            }
            if let Ok(located) = &located
                && history.len() == SETTLE_SAMPLES
                && history.iter().all(|sample| *sample == history[0])
            {
                let mut settled = self.settled.borrow_mut();
                settled.insert(target.to_json().to_string(), located.rect);
                if let Some(name) = name {
                    settled.insert(name, located.rect);
                }
                return Outcome::default();
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "settle": target }),
                    actual: located.map_or_else(
                        |error| json!(error.message()),
                        |located| json!(located.rect.pixels()),
                    ),
                    message: Some("the target to stop moving".to_owned()),
                };
                return Outcome::timed_out(input.window, failure, timeout);
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a wheel's own fields plus its repeat"
    )]
    pub(super) async fn wheel_until(
        &self,
        input: &Input<'_>,
        target: &Target,
        at: Option<&At>,
        clicks: u32,
        down: bool,
        modifiers: &[Modifier],
        repeat: &Repeat,
    ) -> Outcome {
        let deadline = Instant::now() + repeat.timeout;
        let mut turns = 0;
        loop {
            match self.check(input, &repeat.predicate) {
                Err(error) => return Outcome::error(error),
                Ok(Ok(())) => return Outcome::default(),
                Ok(Err(_)) if repeat.tries.is_some_and(|tries| turns >= tries) => {
                    return Outcome::default();
                }
                Ok(Err(mut failure)) if repeat.tries.is_none() && Instant::now() >= deadline => {
                    failure.message.clone_from(&repeat.message);
                    return Outcome::timed_out(input.window, failure, repeat.timeout);
                }
                Ok(Err(_)) => {}
            }
            if let Err(error) = input.wheel(target, at, clicks, down, modifiers).await {
                return Outcome::error(error);
            }
            turns += 1;
            if repeat.tries.is_none() {
                glib::timeout_future(POLL_INTERVAL).await;
            }
        }
    }
}
