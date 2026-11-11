// SPDX-License-Identifier: MIT

//! The parity ledger, `tests/qa/parity/slice.toml`: every Columns run of the scenario files
//! the Rune vertical slice is measured by, each `required` on both targets or `pending` with
//! a reason and a blocker (`docs/rune/slice/README.md`). `./scripts/qa-parity.sh` reads the
//! same file to pick the runs of a gate.
//!
//! It also holds the slice's allowances: what the GTK suite already did wrong before the
//! slice and `./scripts/qa-parity.sh slice` lets stand, each by an owner ruling.

use std::{collections::BTreeSet, fs, path::Path};

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

/// A run of the GTK suite that failed some of the time before the slice. When it fails in
/// the whole suite, `./scripts/qa-parity.sh slice` says so and does not fail for it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Flaky {
    pub(crate) scenario: String,
    pub(crate) run: String,
    pub(crate) reason: String,
    /// The document that records the flake, which names the run.
    pub(crate) recorded: String,
}

/// A GTK mutation patch that stopped applying before the slice. `slice` runs every other
/// patch and requires this one to be stale still.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StalePatch {
    pub(crate) patch: String,
    pub(crate) reason: String,
    /// The document that records the stale patch, which names its file.
    pub(crate) recorded: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ledger {
    #[serde(rename = "run", default)]
    pub(crate) runs: Vec<Entry>,
    #[serde(default)]
    pub(crate) flaky: Vec<Flaky>,
    #[serde(rename = "stale_patch", default)]
    pub(crate) stale_patches: Vec<StalePatch>,
}

/// What an allowance is checked against: the GTK suite as the runner plans it.
#[derive(Debug, Default)]
pub(crate) struct Suite {
    /// Every `<scenario>/<run>`, in every mode.
    pub(crate) runs: BTreeSet<String>,
    /// Every mutation patch a scenario names.
    pub(crate) mutations: BTreeSet<String>,
}

impl Suite {
    pub(crate) fn planned() -> Self {
        let mut suite = Self::default();
        for path in scenario_files() {
            let scenario =
                load_scenario(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let name = &scenario.header.scenario;
            suite.runs.extend(
                runs(&scenario.header)
                    .into_iter()
                    .map(|run| format!("{name}/{}", run.label)),
            );
            suite.mutations.extend(scenario.header.mutation.clone());
        }
        suite
    }
}

/// The text of a document an allowance says it is recorded in, if the repository has it.
pub(crate) fn document(path: &str) -> Option<String> {
    fs::read_to_string(Path::new(REPOSITORY).join(path)).ok()
}

/// A run of the set as the runner plans it: scenario, run label and gate.
pub(crate) type Planned = (String, String, Gate);

impl Ledger {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|error| format!("{LEDGER}: {error}"))
    }

    pub(crate) fn load() -> Result<Self, String> {
        let path = Path::new(REPOSITORY).join(LEDGER);
        let text = fs::read_to_string(&path).map_err(|error| format!("{LEDGER}: {error}"))?;
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

    /// Everything wrong with the allowances, one line each. `planned` is the parity set,
    /// `suite` the whole GTK suite and `document` reads what an allowance cites.
    pub(crate) fn allowance_problems(
        &self,
        planned: &[Planned],
        suite: &Suite,
        document: impl Fn(&str) -> Option<String>,
    ) -> Vec<String> {
        let flaky = self
            .flaky
            .iter()
            .map(|flaky| (flaky.cited(), flaky.misplaced(planned, suite)));
        let stale = self
            .stale_patches
            .iter()
            .map(|stale| (stale.cited(), stale.misplaced(suite)));
        let mut listed = BTreeSet::new();
        flaky
            .chain(stale)
            .filter_map(|(cited, misplaced)| {
                let twice =
                    (!listed.insert(cited.name.clone())).then(|| "is allowed twice".to_owned());
                let problem = twice.or(misplaced).or_else(|| cited.unfounded(&document))?;
                Some(format!("{} {problem}", cited.name))
            })
            .collect()
    }
}

impl Flaky {
    pub(crate) fn name(&self) -> String {
        format!("{}/{}", self.scenario, self.run)
    }

    fn cited(&self) -> Cited<'_> {
        Cited {
            name: self.name(),
            reason: &self.reason,
            recorded: &self.recorded,
        }
    }

    /// What is wrong with the run named: none of the suite's, or one a gate decides.
    fn misplaced(&self, planned: &[Planned], suite: &Suite) -> Option<String> {
        let in_the_set = planned
            .iter()
            .any(|(scenario, run, _)| *scenario == self.scenario && *run == self.run);
        if !suite.runs.contains(&self.name()) {
            Some("is no run of the GTK suite".to_owned())
        } else if in_the_set {
            Some("is a run of the parity set, which a gate decides".to_owned())
        } else {
            None
        }
    }
}

impl StalePatch {
    fn cited(&self) -> Cited<'_> {
        Cited {
            name: format!("{}.patch", self.patch),
            reason: &self.reason,
            recorded: &self.recorded,
        }
    }

    fn misplaced(&self, suite: &Suite) -> Option<String> {
        (!suite.mutations.contains(&self.patch))
            .then(|| "is no mutation patch a scenario names".to_owned())
    }
}

/// What every allowance owes: why it stands, and a document that names it.
struct Cited<'a> {
    name: String,
    reason: &'a str,
    recorded: &'a str,
}

impl Cited<'_> {
    fn unfounded(&self, document: &impl Fn(&str) -> Option<String>) -> Option<String> {
        if self.reason.trim().is_empty() {
            return Some("is allowed without a reason".to_owned());
        }
        let named = document(self.recorded).is_some_and(|text| text.contains(&self.name));
        (!named).then(|| format!("is not recorded in {:?}", self.recorded))
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
