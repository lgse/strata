// SPDX-License-Identifier: MIT

//! `adopt` and `activate_window`: which window the steps drive.

use std::time::{Duration, Instant};

use gtk::{glib, prelude::*};
use serde_json::{Value, json};

use super::{
    Controller, Outcome, POLL_INTERVAL,
    input::Injector,
    protocol::{Command, Failure},
    settle, targets,
};

pub(super) struct Adoption<'a> {
    pub(super) title: &'a str,
    /// `activate_window`: the only window with the title, raised and given the keyboard.
    pub(super) activate: bool,
    pub(super) timeout: Duration,
    pub(super) message: Option<&'a str>,
}

impl Controller {
    pub(super) async fn adopt(&self, adoption: &Adoption<'_>) -> Outcome {
        let deadline = Instant::now() + adoption.timeout;
        let adopted = match adoptable(adoption, deadline).await {
            Ok(window) => window,
            Err(actual) => return adoption.timed_out(actual),
        };
        self.main.set(Some(&adopted));
        if adoption.activate
            && let Err(error) = give_keyboard(self.injector.as_ref(), &adopted, deadline).await
        {
            return match error {
                Activation::Broken(error) => Outcome::error(error),
                Activation::Inactive => adoption.timed_out(json!({ "active": false })),
            };
        }
        settle(Some(&adopted)).await;
        Outcome::default()
    }
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

    /// A window to activate must be the only one with its title, so the step means one window
    /// on every toolkit.
    fn chosen(&self, windows: Vec<gtk::Window>) -> Option<gtk::Window> {
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

async fn adoptable(adoption: &Adoption<'_>, deadline: Instant) -> Result<gtk::Window, Value> {
    loop {
        let windows = targets::windows_titled(adoption.title);
        let showing = windows.len();
        if let Some(window) = adoption.chosen(windows) {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(adoption.missing(showing));
        }
        glib::timeout_future(POLL_INTERVAL).await;
    }
}

enum Activation {
    Broken(String),
    Inactive,
}

/// On a bare X server `present` already asks for the focus; the injector sets it as well, at
/// the current time, so the switch does not rest on the timestamp GDK sends with it.
async fn give_keyboard(
    injector: Option<&Injector>,
    window: &gtk::Window,
    deadline: Instant,
) -> Result<(), Activation> {
    let injector = injector.ok_or_else(|| Activation::Broken("no input connection".to_owned()))?;
    window.present();
    let surface = window
        .surface()
        .ok_or_else(|| Activation::Broken("the window has no surface".to_owned()))?;
    injector.activate(&surface).map_err(Activation::Broken)?;
    while !window.is_active() {
        if Instant::now() >= deadline {
            return Err(Activation::Inactive);
        }
        glib::timeout_future(POLL_INTERVAL).await;
    }
    Ok(())
}
