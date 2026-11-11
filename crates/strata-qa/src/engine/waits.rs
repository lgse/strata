// SPDX-License-Identifier: MIT

//! The steps that wait for the window to come to rest, or act until a predicate holds.

use std::time::{Duration, Instant};

use serde_json::json;

use super::{
    MENU_RETRY_AFTER, POLL_INTERVAL, SETTLE_SAMPLES, lookup::windows_titled, pointer::Aimed,
    scope::Scope, wanted::has_state,
};
use crate::{
    Aim, At, Backend, DEFAULT_WAIT, Failure, LookupError, Modifier, Outcome, Predicate, Repeat,
    StateName, Target,
};

/// Waits for a drawn window with the title and makes it the one steps act on.
pub(super) async fn adopt<B: Backend>(
    backend: &B,
    title: &str,
    timeout: Duration,
    message: Option<&str>,
) -> Outcome {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(window) = windows_titled(backend, title).into_iter().next() {
            backend.adopt(&window);
            backend.settle(&window).await;
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
        backend.sleep(POLL_INTERVAL).await;
    }
}

/// The wheel turns a `wheel` step repeats until its predicate holds.
pub(super) struct Turn<'a> {
    pub(super) target: &'a Target,
    pub(super) at: Option<&'a At>,
    pub(super) clicks: u32,
    pub(super) down: bool,
    pub(super) modifiers: &'a [Modifier],
}

impl<B: Backend> Scope<'_, B> {
    /// A late startup focus change can close the menu as it opens, so it is re-clicked each second.
    pub(super) async fn open_menu(
        &self,
        target: &Target,
        until: &Predicate,
        message: Option<String>,
        open_states: &[StateName],
    ) -> Outcome {
        let aim = Aimed::plain(target);
        if let Err(error) = self.click(&aim, 1, 1, &[]).await {
            return Outcome::error(error);
        }
        let mut clicked = Instant::now();
        let deadline = clicked + DEFAULT_WAIT;
        loop {
            match self.check(until) {
                Err(error) => return Outcome::error(error),
                Ok(Ok(())) => return Outcome::default(),
                Ok(Err(mut failure)) if Instant::now() >= deadline => {
                    failure.message = message;
                    return self.timed_out(failure, DEFAULT_WAIT);
                }
                Ok(Err(_)) => {}
            }
            // GTK never reports `checked` for a toggle button, so re-click while the menu is shut.
            let open = self
                .locate_now(target, None, Aim::Point)
                .is_ok_and(|located| {
                    open_states
                        .iter()
                        .any(|state| has_state(self.backend, &located.node, *state))
                });
            if clicked.elapsed() > MENU_RETRY_AFTER && !open {
                if let Err(error) = self.click(&aim, 1, 1, &[]).await {
                    return Outcome::error(error);
                }
                clicked = Instant::now();
            }
            self.backend.sleep(POLL_INTERVAL).await;
        }
    }

    /// Waits until the target's box is the same at three polls running, and keeps the box
    /// under the target's own spelling and under `name`.
    pub async fn settle_target(
        &self,
        target: &Target,
        timeout: Duration,
        name: Option<String>,
    ) -> Outcome {
        let deadline = Instant::now() + timeout;
        let mut history: Vec<Option<[i64; 4]>> = Vec::new();
        loop {
            let located = self.locate_now(target, None, Aim::Point);
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
                let mut settled = self.kept.boxes.borrow_mut();
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
                return self.timed_out(failure, timeout);
            }
            self.backend.sleep(POLL_INTERVAL).await;
        }
    }

    pub(super) async fn wheel_until(&self, turn: &Turn<'_>, repeat: &Repeat) -> Outcome {
        let deadline = Instant::now() + repeat.timeout;
        let mut turns = 0;
        loop {
            match self.check(&repeat.predicate) {
                Err(error) => return Outcome::error(error),
                Ok(Ok(())) => return Outcome::default(),
                Ok(Err(_)) if repeat.tries.is_some_and(|tries| turns >= tries) => {
                    return Outcome::default();
                }
                Ok(Err(mut failure)) if repeat.tries.is_none() && Instant::now() >= deadline => {
                    failure.message.clone_from(&repeat.message);
                    return self.timed_out(failure, repeat.timeout);
                }
                Ok(Err(_)) => {}
            }
            if let Err(error) = self
                .wheel(turn.target, turn.at, turn.clicks, turn.down, turn.modifiers)
                .await
            {
                return Outcome::error(error);
            }
            turns += 1;
            if repeat.tries.is_none() {
                self.backend.sleep(POLL_INTERVAL).await;
            }
        }
    }
}
