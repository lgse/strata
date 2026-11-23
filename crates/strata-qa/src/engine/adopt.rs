// SPDX-License-Identifier: MIT

//! `adopt` and `activate_window`: which window the steps drive.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{POLL_INTERVAL, lookup::windows_titled};
use crate::{Backend, Command, Failure, Outcome};

pub(super) struct Adoption<'a> {
    title: &'a str,
    /// `activate_window`: the only window with the title, raised and given the keyboard.
    activate: bool,
    timeout: Duration,
    message: Option<&'a str>,
}

impl<'a> Adoption<'a> {
    pub(super) fn of(command: &'a Command) -> Option<Self> {
        let Command::Adopt {
            title,
            activate,
            timeout,
            message,
        } = command
        else {
            return None;
        };
        Some(Self {
            title,
            activate: *activate,
            timeout: *timeout,
            message: message.as_deref(),
        })
    }

    fn chosen<N>(&self, windows: Vec<N>) -> Option<N> {
        if self.activate && windows.len() != 1 {
            return None;
        }
        windows.into_iter().next()
    }

    /// What the timeout found: how many windows had the title, when one had to be the only one.
    fn missing(&self, showing: usize) -> Value {
        if self.activate {
            json!({ "windows": showing })
        } else {
            json!(false)
        }
    }

    fn predicate(&self) -> Value {
        if self.activate {
            json!({ "activate_window": { "title": self.title } })
        } else {
            json!({ "adopt": self.title })
        }
    }

    fn timed_out(&self, actual: Value) -> Outcome {
        Outcome {
            failures: vec![Failure {
                predicate: self.predicate(),
                actual,
                message: self.message.map(str::to_owned),
            }],
            error: Some(format!("timed out after {} ms", self.timeout.as_millis())),
            ..Outcome::default()
        }
    }
}

pub(super) async fn adopt<B: Backend>(backend: &B, adoption: &Adoption<'_>) -> Outcome {
    let deadline = Instant::now() + adoption.timeout;
    let window = match adoptable(backend, adoption, deadline).await {
        Ok(window) => window,
        Err(actual) => return adoption.timed_out(actual),
    };
    backend.adopt(&window);
    if adoption.activate {
        match backend.activate_window(&window, deadline).await {
            Ok(true) => {}
            Ok(false) => return adoption.timed_out(json!({ "active": false })),
            Err(error) => return Outcome::error(error),
        }
    }
    backend.settle(&window).await;
    Outcome::default()
}

/// The window the step takes once one is showing, or what the timeout found.
async fn adoptable<B: Backend>(
    backend: &B,
    adoption: &Adoption<'_>,
    deadline: Instant,
) -> Result<B::Node, Value> {
    loop {
        let windows = windows_titled(backend, adoption.title);
        let showing = windows.len();
        if let Some(window) = adoption.chosen(windows) {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(adoption.missing(showing));
        }
        backend.sleep(POLL_INTERVAL).await;
    }
}
