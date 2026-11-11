// SPDX-License-Identifier: MIT

//! The parity ledger, `tests/qa/parity/slice.toml`: every Columns run of the scenario files
//! the Rune vertical slice is measured by, each `required` on both targets or `pending` with
//! a reason and a blocker (`docs/rune/slice/README.md`). `./scripts/qa-parity.sh` reads the
//! same file to pick the runs of a gate.

use std::{collections::BTreeSet, path::Path};

use serde::Deserialize;

use crate::{
    REPOSITORY,
    scenario::{load_scenario, scenario_files},
    variables::runs,
};

pub(crate) const LEDGER: &str = "tests/qa/parity/slice.toml";
const COLUMNS: &str = "columns";
/// Gate G1 is this one run, taken out of the first of gate G2's folders.
const FIRST_LIGHT: (&str, &str) = ("arrow-keys-move-focus", "columns-arrows");
/// The folders under `tests/qa/scenarios` whose Columns runs are the set, and their gate.
const SET: [(&str, Gate); 7] = [
    ("keyboard_navigation", Gate::G2),
    ("selection", Gate::G2),
    ("escape_selection", Gate::G2),
    ("background_selection", Gate::G2),
    ("hidden_files", Gate::G2),
    ("startup_arguments", Gate::G2),
    ("inline_renaming", Gate::G3),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) enum Gate {
    G1,
    G2,
    G3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Required,
    Pending,
}

/// Why a pending run does not pass on the Rune target yet. `Scope` is a part of the
/// interface the slice leaves out; `Component` is a component that lacks or differs in a
/// behaviour (`docs/rune/slice/README.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Reason {
    Harness,
    Component,
    Rune,
    Platform,
    Toolkit,
    Deviation,
    Scope,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub(crate) scenario: String,
    pub(crate) run: String,
    pub(crate) gate: Gate,
    pub(crate) status: Status,
    pub(crate) reason: Option<Reason>,
    pub(crate) blocker: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ledger {
    #[serde(rename = "run", default)]
    pub(crate) runs: Vec<Entry>,
}

/// A run of the set as the runner plans it: scenario, run label and gate.
pub(crate) type Planned = (String, String, Gate);

impl Ledger {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|error| format!("{LEDGER}: {error}"))
    }

    pub(crate) fn load() -> Result<Self, String> {
        let path = Path::new(REPOSITORY).join(LEDGER);
        let text = std::fs::read_to_string(&path).map_err(|error| format!("{LEDGER}: {error}"))?;
        Self::parse(&text)
    }

    /// Everything wrong with the ledger against the runs the runner plans, one line each.
    pub(crate) fn problems(&self, planned: &[Planned]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut listed = BTreeSet::new();
        for entry in &self.runs {
            let name = format!("{}/{}", entry.scenario, entry.run);
            if !listed.insert((entry.scenario.as_str(), entry.run.as_str())) {
                problems.push(format!("{name} is listed twice"));
            }
            let found = entry.misplaced(planned).or_else(|| entry.unexplained());
            problems.extend(found.map(|problem| format!("{name} {problem}")));
        }
        let unlisted = planned
            .iter()
            .filter(|(scenario, run, _)| !listed.contains(&(scenario.as_str(), run.as_str())));
        problems
            .extend(unlisted.map(|(scenario, run, _)| format!("{scenario}/{run} is not listed")));
        problems
    }
}

impl Entry {
    /// What is wrong with where the entry stands: no such run, or another gate's.
    fn misplaced(&self, planned: &[Planned]) -> Option<String> {
        let gate = planned
            .iter()
            .find(|(scenario, run, _)| *scenario == self.scenario && *run == self.run)
            .map(|(_, _, gate)| *gate);
        match gate {
            None => Some("is no Columns run of the set".to_owned()),
            Some(gate) if gate != self.gate => Some(format!("belongs to gate {gate:?}")),
            Some(_) => None,
        }
    }

    /// A pending run says why and what blocks it; a required run has nothing to explain.
    fn unexplained(&self) -> Option<String> {
        let blocked = self
            .blocker
            .as_deref()
            .is_some_and(|blocker| !blocker.trim().is_empty());
        let problem = match self.status {
            Status::Pending if self.reason.is_none() => "is pending without a reason",
            Status::Pending if !blocked => "is pending without a blocker",
            Status::Required if self.reason.is_some() || self.blocker.is_some() => {
                "is required, so it has no reason or blocker"
            }
            _ => return None,
        };
        Some(problem.to_owned())
    }
}

/// The Columns runs of the set's scenario files, as the runner plans them.
pub(crate) fn planned() -> Vec<Planned> {
    let mut planned = Vec::new();
    for path in scenario_files() {
        let folder = path.parent().and_then(|folder| folder.file_name());
        let gate = SET
            .iter()
            .find(|(name, _)| folder.is_some_and(|folder| folder == *name))
            .map(|(_, gate)| *gate);
        let Some(gate) = gate else {
            continue;
        };
        let scenario =
            load_scenario(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let name = &scenario.header.scenario;
        for run in runs(&scenario.header) {
            if run.mode != COLUMNS {
                continue;
            }
            let first_light = (name.as_str(), run.label.as_str()) == FIRST_LIGHT;
            let gate = if first_light { Gate::G1 } else { gate };
            planned.push((name.clone(), run.label, gate));
        }
    }
    planned
}
