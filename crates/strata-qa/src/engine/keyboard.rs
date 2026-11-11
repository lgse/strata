// SPDX-License-Identifier: MIT

//! Walks the keyboard cursor to an entry, one key at a time.

use std::time::Instant;

use serde_json::json;

use super::{KEYBOARD_SELECT_STEPS, POLL_INTERVAL, scope::Scope};
use crate::{Backend, Chord, DEFAULT_WAIT, Failure, Observation, Outcome};

impl<B: Backend> Scope<'_, B> {
    pub(super) async fn press(&self, chord: &Chord) -> Result<(), String> {
        self.backend.chord(&self.window, chord).await
    }

    async fn wait_for(
        &self,
        name: &str,
        holds: impl Fn(&Observation) -> bool,
        message: String,
    ) -> Result<(), Outcome> {
        let deadline = Instant::now() + DEFAULT_WAIT;
        loop {
            let observation = self.scene().observation;
            if holds(&observation) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "select_with_keyboard": name }),
                    actual: json!(observation.focused_entry),
                    message: Some(message),
                };
                return Err(self.timed_out(failure, DEFAULT_WAIT));
            }
            self.backend.sleep(POLL_INTERVAL).await;
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
        let icons = self.scene().observation.view_mode() == Some("Icons");
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
            let previous = self.scene().observation.focused_entry;
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
                actual: json!(self.scene().observation.focused_entry),
                message: Some(format!("the keyboard cursor to reach {name:?}")),
            }],
            error: Some(format!("the keyboard cursor never reached {name:?}")),
            ..Outcome::default()
        }
    }
}
