// SPDX-License-Identifier: MIT

//! Walks the keyboard cursor to an entry, one key at a time.

use std::time::Instant;

use serde_json::json;

use super::{KEYBOARD_SELECT_STEPS, POLL_INTERVAL, scope::Scope};
use crate::{Backend, Chord, DEFAULT_WAIT, Failure, Observation, Outcome};

enum StepResult {
    Reached(Outcome),
    Continue,
    Failed(Outcome),
}

struct CursorBefore {
    focused: Option<String>,
    listed: Vec<String>,
}

fn target_is_above(listed: &[String], name: &str, focused: Option<&str>) -> bool {
    let focused = focused.and_then(|cur| listed.iter().position(|entry| entry == cur));
    let target = listed.iter().position(|entry| entry == name);
    matches!((target, focused), (Some(t), Some(f)) if t < f)
}

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

    async fn rehome(&self, name: &str, home: &Chord) -> Result<(), Outcome> {
        self.press(home).await.map_err(Outcome::error)?;
        self.wait_for(
            name,
            |observation| observation.focused_entry.is_some(),
            "the keyboard cursor".to_owned(),
        )
        .await
    }

    async fn confirm_listed(&self, name: &str) -> Outcome {
        match self
            .wait_for(
                name,
                |observation| observation.entry(name, None).is_some(),
                format!("the entry {name:?} to appear"),
            )
            .await
        {
            Ok(()) => Outcome::default(),
            Err(outcome) => outcome,
        }
    }

    async fn advance_cursor(
        &self,
        name: &str,
        before: CursorBefore,
        (next, home): (&Chord, &Chord),
    ) -> StepResult {
        if let Err(error) = self.press(next).await {
            return StepResult::Failed(Outcome::error(error));
        }
        if let Err(outcome) = self
            .wait_for(
                name,
                |obs| obs.focused_entry != before.focused || obs.entry_names(None) != before.listed,
                format!("the cursor to move past {:?}", before.focused),
            )
            .await
        {
            return StepResult::Failed(outcome);
        }
        if self.scene().observation.focused_entry == before.focused
            && let Err(outcome) = self.rehome(name, home).await
        {
            return StepResult::Failed(outcome);
        }
        StepResult::Continue
    }

    async fn step_toward(&self, name: &str, next: &Chord, home: &Chord) -> StepResult {
        let observation = self.scene().observation;
        if observation.focused_entry.as_deref() == Some(name) {
            return StepResult::Reached(self.confirm_listed(name).await);
        }
        let listed = observation.entry_names(None);
        let focused = observation.focused_entry;
        if target_is_above(&listed, name, focused.as_deref()) {
            return match self.rehome(name, home).await {
                Ok(()) => StepResult::Continue,
                Err(outcome) => StepResult::Failed(outcome),
            };
        }
        let before = CursorBefore { focused, listed };
        self.advance_cursor(name, before, (next, home)).await
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
        if let Err(outcome) = self.rehome(name, &home).await {
            return outcome;
        }
        for _ in 0..KEYBOARD_SELECT_STEPS {
            match self.step_toward(name, &next, &home).await {
                StepResult::Reached(outcome) | StepResult::Failed(outcome) => return outcome,
                StepResult::Continue => {}
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
