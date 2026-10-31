// SPDX-License-Identifier: MIT

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    Application, COMMAND_TIMEOUT, execute,
    fixture::manifest,
    harness, require_container,
    scenario::{Scenario, load_scenario},
    steps::run_steps,
    variables::{Run, placeholders, run_name, runs, variables},
};

const STATE_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const STATE_SETTLE_INTERVAL: Duration = Duration::from_millis(250);

/// `errors` are the harness's own problems and fail a run even when it is `known_bug`;
/// `failures` are the scenario's.
#[derive(Default)]
pub(crate) struct Outcome {
    pub(crate) errors: Vec<String>,
    pub(crate) failures: Vec<(Option<usize>, String)>,
    pub(crate) artifacts: Option<PathBuf>,
    pub(crate) last_tree: Option<String>,
    pub(crate) transcript: String,
}

impl Outcome {
    pub(crate) fn failure_texts(&self) -> impl Iterator<Item = &String> {
        self.failures.iter().map(|(_, text)| text)
    }
}

pub(crate) fn run_scenario(path: &Path, label: &str) -> harness::Outcome {
    require_container();
    let scenario =
        load_scenario(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let manifest = manifest();
    let run = runs(&scenario.header)
        .into_iter()
        .find(|run| run.label == label)
        .unwrap_or_else(|| panic!("{}: no run {label}", path.display()));
    if let Err(error) = run_steps(&scenario, &run, &variables(&run, &placeholders())) {
        panic!("{}/{}: {error}", scenario.header.scenario, run.label);
    }
    let outcome = execute(&scenario, &run, &manifest);
    let header = &scenario.header;
    let name = run_name(&header.scenario, &run);
    let details = |heading: &str| {
        let mut lines: Vec<String> = outcome
            .errors
            .iter()
            .chain(outcome.failure_texts())
            .cloned()
            .collect();
        if let Some(artifacts) = &outcome.artifacts {
            lines.push(format!("artifacts: {}", artifacts.display()));
        }
        harness::Outcome::Failed(format!("{name} {heading}:\n  {}", lines.join("\n  ")))
    };
    if !outcome.errors.is_empty() {
        return details("could not run");
    }
    match (&header.known_bug, outcome.failures.is_empty()) {
        (None, true) => harness::Outcome::Passed(Some(format!("PASS {name}"))),
        (None, false) => details("failed"),
        (Some(bug), true) => harness::Outcome::Failed(format!(
            "{name} passes but is marked known_bug {bug}; remove the marker"
        )),
        (Some(bug), false)
            if outcome
                .failures
                .iter()
                .all(|(line, _)| *line == header.known_bug_step) =>
        {
            harness::Outcome::Passed(Some(format!(
                "KNOWN BUG {name} still fails as expected ({bug})"
            )))
        }
        (Some(_), false) => details(&format!(
            "is marked known_bug at line {} but failed elsewhere",
            header.known_bug_step.unwrap_or_default()
        )),
    }
}

/// Two equal reads in a row, so a late selection or focus change is not compared half done.
pub(crate) fn settled_state(application: &mut Application) -> Value {
    let deadline = Instant::now() + STATE_SETTLE_TIMEOUT;
    let mut previous = Value::Null;
    loop {
        let Ok(reply) = application.send(&json!({ "query": "state" }), COMMAND_TIMEOUT) else {
            return previous;
        };
        let state = reply["app"].clone();
        if state == previous || Instant::now() >= deadline {
            return state;
        }
        previous = state;
        std::thread::sleep(STATE_SETTLE_INTERVAL);
    }
}

pub(crate) fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default() + "\n"
}

pub(crate) fn run_result(
    scenario: &Scenario,
    run: &Run,
    steps: &[(usize, Value)],
    outcome: &Outcome,
) -> Value {
    let failures: Vec<Value> = outcome
        .failures
        .iter()
        .map(|(line, text)| {
            let step = line
                .and_then(|line| steps.iter().find(|(number, _)| *number == line))
                .map_or(Value::Null, |(_, step)| step.clone());
            json!({ "line": line, "step": step, "text": text })
        })
        .collect();
    json!({
        "scenario": scenario.header.scenario,
        "run": run.label,
        "mode": run.mode,
        "errors": outcome.errors,
        "failures": failures,
    })
}
