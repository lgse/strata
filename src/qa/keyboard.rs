// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::glib;
use serde_json::json;

use super::{
    EVENT_GAP, KEYBOARD_SELECT_STEPS, Outcome, POLL_INTERVAL,
    driver::Input,
    input::{self, Event},
    protocol::{self, Chord, Failure, Modifier},
    state,
};

impl Input<'_> {
    async fn key(&self, keycode: u8, pressed: bool) -> Result<(), String> {
        self.emit(Event::Key { keycode, pressed }, EVENT_GAP).await
    }

    pub(super) async fn raw_key(&self, name: &str, pressed: bool) -> Result<(), String> {
        let keysym = match Modifier::parse(name) {
            Ok(modifier) => input::modifier_keysym(modifier)?,
            Err(_) => input::keysym(name)?,
        };
        let (keycode, _) = self.injector.keycode(keysym)?;
        self.emit(Event::Key { keycode, pressed }, Duration::ZERO)
            .await
    }

    pub(super) async fn hold(&self, modifiers: &[Modifier]) -> Result<Vec<u8>, String> {
        let keycodes = modifiers
            .iter()
            .map(|modifier| {
                let (keycode, _) = self.injector.keycode(input::modifier_keysym(*modifier)?)?;
                Ok(keycode)
            })
            .collect::<Result<Vec<u8>, String>>()?;
        let mut held = Vec::new();
        for keycode in keycodes {
            if let Err(error) = self.key(keycode, true).await {
                self.release(&held).await;
                return Err(error);
            }
            held.push(keycode);
        }
        Ok(held)
    }

    pub(super) async fn release(&self, held: &[u8]) {
        for keycode in held.iter().rev() {
            let _ = self.key(*keycode, false).await;
        }
    }

    pub(super) async fn press(&self, chord: &Chord) -> Result<(), String> {
        let (keycode, needs_shift) = self.injector.keycode(input::keysym(&chord.key)?)?;
        if needs_shift {
            self.injector
                .keycode(input::modifier_keysym(Modifier::Shift)?)?;
        }
        let held = self.hold(&chord.modifiers).await?;
        let tapped = self.tap_key(keycode, needs_shift).await;
        self.release(&held).await;
        tapped
    }

    /// The server times the release `EVENT_GAP` after the press: waiting for a frame could hold
    /// the key past the X server's 660 ms autorepeat delay while the app is busy.
    async fn tap_key(&self, keycode: u8, needs_shift: bool) -> Result<(), String> {
        let shift = if needs_shift {
            self.hold(&[Modifier::Shift]).await?
        } else {
            Vec::new()
        };
        let up = Event::Key {
            keycode,
            pressed: false,
        };
        let mut result = self.injector.send(Event::Key {
            keycode,
            pressed: true,
        });
        if result.is_ok() {
            result = match self.injector.send_after(up, EVENT_GAP) {
                Ok(()) => {
                    glib::timeout_future(EVENT_GAP).await;
                    self.injector.sync()
                }
                // A key left down would autorepeat until the next press.
                Err(error) => self.injector.send(up).and(Err(error)),
            };
            self.pause(EVENT_GAP).await;
        }
        self.release(&shift).await;
        result
    }

    async fn wait_for(
        &self,
        name: &str,
        holds: impl Fn(&state::Observation) -> bool,
        message: String,
    ) -> Result<(), Outcome> {
        let deadline = Instant::now() + protocol::DEFAULT_WAIT;
        loop {
            let observation = state::scan(self.window).observation;
            if holds(&observation) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "select_with_keyboard": name }),
                    actual: json!(observation.focused_entry),
                    message: Some(message),
                };
                return Err(Outcome::timed_out(
                    self.window,
                    failure,
                    protocol::DEFAULT_WAIT,
                ));
            }
            glib::timeout_future(POLL_INTERVAL).await;
        }
    }

    pub(super) async fn select_with_keyboard(&self, name: &str, step: Option<&str>) -> Outcome {
        let mut outcome = self.walk_to(name).await;
        if let Some(step) = step {
            for failure in &mut outcome.failures {
                failure.message = Some(match failure.message.take() {
                    Some(detail) => format!("{step}: {detail}"),
                    None => step.to_owned(),
                });
            }
        }
        outcome
    }

    async fn walk_to(&self, name: &str) -> Outcome {
        if let Err(outcome) = self
            .wait_for(
                name,
                |observation| observation.view_mode().is_some(),
                "a browser pane to be on screen".to_owned(),
            )
            .await
        {
            return outcome;
        }
        let icons = state::scan(self.window).observation.view_mode() == Some("Icons");
        let next = Chord {
            modifiers: Vec::new(),
            key: if icons { "Right" } else { "Down" }.to_owned(),
        };
        let home = Chord {
            modifiers: Vec::new(),
            key: "Home".to_owned(),
        };
        if let Err(error) = self.press(&home).await {
            return Outcome::error(error);
        }
        if let Err(outcome) = self
            .wait_for(
                name,
                |observation| observation.focused_entry.is_some(),
                "the keyboard cursor".to_owned(),
            )
            .await
        {
            return outcome;
        }
        for _ in 0..KEYBOARD_SELECT_STEPS {
            let previous = state::scan(self.window).observation.focused_entry;
            if previous.as_deref() == Some(name) {
                return match self
                    .wait_for(
                        name,
                        |observation| observation.entry(name, None).is_some(),
                        format!("the entry {name:?} to appear"),
                    )
                    .await
                {
                    Ok(()) => Outcome::default(),
                    Err(outcome) => outcome,
                };
            }
            if let Err(error) = self.press(&next).await {
                return Outcome::error(error);
            }
            if let Err(outcome) = self
                .wait_for(
                    name,
                    |observation| observation.focused_entry != previous,
                    format!("the cursor to move past {previous:?}"),
                )
                .await
            {
                return outcome;
            }
        }
        Outcome {
            failures: vec![Failure {
                predicate: json!({ "select_with_keyboard": name }),
                actual: json!(state::scan(self.window).observation.focused_entry),
                message: Some(format!("the keyboard cursor to reach {name:?}")),
            }],
            error: Some(format!("the keyboard cursor never reached {name:?}")),
            ..Outcome::default()
        }
    }
}
