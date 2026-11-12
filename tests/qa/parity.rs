// SPDX-License-Identifier: MIT

//! The parity ledger's runs, `tests/qa/parity/<folder>.toml`: one file for each folder under
//! `tests/qa/scenarios`, listing every run of the folder once with the work package that
//! owns it. A run is `required` on both targets, `pending` with a reason and a blocker, or
//! `excluded` by an owner ruling (`docs/testing.md`, "The Rune target and the parity
//! ledger"). `./scripts/qa-parity.sh` reads the same files to pick the runs of a selection.
//!
//! The ledger's one other file holds the slice's allowances: `allowances.rs`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::RangeInclusive,
    path::Path,
};

use serde::Deserialize;

use crate::{
    REPOSITORY, allowances,
    scenario::{load_scenario, scenario_files},
    variables::{Run, runs},
};

pub(crate) const LEDGER: &str = "tests/qa/parity";
const COLUMNS: &str = "columns";
/// Gate G1 is this one run, taken out of the first of gate G2's folders.
const FIRST_LIGHT: (&str, &str) = ("arrow-keys-move-focus", "columns-arrows");
/// The folders whose Columns runs are the slice's set, and their gate
/// (`docs/rune/slice/README.md`). No other run has a gate.
const SET: [(&str, Gate); 7] = [
    ("keyboard_navigation", Gate::G2),
    ("selection", Gate::G2),
    ("escape_selection", Gate::G2),
    ("background_selection", Gate::G2),
    ("hidden_files", Gate::G2),
    ("startup_arguments", Gate::G2),
    ("inline_renaming", Gate::G3),
];
/// The slice's packages, which own the runs they took to `required` and no other: V7 gate
/// G1's, V8 gate G2's and V9 gate G3's (`docs/rune/slice/README.md`).
const SLICE: [(&str, Gate); 3] = [("V7", Gate::G1), ("V8", Gate::G2), ("V9", Gate::G3)];
/// What to do about a run the ledger does not list, for whoever meets the failure.
pub(crate) const HELP: &str = "A run that is not listed goes into tests/qa/parity/<its \
    folder>.toml, at its place in the runner's order, as pending on scope under its package: \
    docs/testing.md, \"Who owns a run\", says which.";
/// The parity plan's work packages, by track letter: `H0` to `H5`, `B1` to `B3`, `F1` to
/// `F5`, `A1` to `A21` and `E1` to `E4`.
const TRACKS: [(char, RangeInclusive<u8>); 5] = [
    ('H', 0..=5),
    ('B', 1..=3),
    ('F', 1..=5),
    ('A', 1..=21),
    ('E', 1..=4),
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
    Excluded,
}

/// Where a run runs on the Rune target: the offscreen host, or a real window in a
/// headless compositor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Host {
    Offscreen,
    Session,
}

/// Why a pending run does not pass on the Rune target yet. `Scope` is a part of the
/// interface no package has ported yet; `Component` is a component that lacks or differs
/// in a behaviour (`docs/testing.md`).
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
    /// The owner: only this package edits the entry.
    pub(crate) package: String,
    pub(crate) gate: Option<Gate>,
    pub(crate) status: Status,
    pub(crate) host: Option<Host>,
    pub(crate) reason: Option<Reason>,
    pub(crate) blocker: Option<String>,
    pub(crate) ruling: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(rename = "run", default)]
    runs: Vec<Entry>,
}

/// Every ledger file, by the scenario folder it is named for.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    pub(crate) folders: BTreeMap<String, Vec<Entry>>,
}

/// A run as the runner plans it. Only a Columns run of the slice's set has a gate.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Planned {
    pub(crate) folder: String,
    pub(crate) scenario: String,
    pub(crate) run: String,
    pub(crate) gate: Option<Gate>,
}

impl Ledger {
    /// Reads `<folder>.toml` as the ledger of that folder.
    pub(crate) fn add(&mut self, folder: &str, text: &str) -> Result<(), String> {
        let file: File =
            toml::from_str(text).map_err(|error| format!("{LEDGER}/{folder}.toml: {error}"))?;
        self.folders.insert(folder.to_owned(), file.runs);
        Ok(())
    }

    pub(crate) fn load() -> Result<Self, String> {
        let directory = Path::new(REPOSITORY).join(LEDGER);
        let allowances = Path::new(REPOSITORY).join(allowances::ALLOWANCES);
        let failed = |error: std::io::Error| format!("{LEDGER}: {error}");
        let mut ledger = Self::default();
        for file in fs::read_dir(&directory).map_err(failed)? {
            let path = file.map_err(failed)?.path();
            let listed = path
                .extension()
                .is_some_and(|extension| extension == "toml")
                && path != allowances;
            let folder = path.file_stem().and_then(|stem| stem.to_str());
            if let (true, Some(folder)) = (listed, folder) {
                let text = fs::read_to_string(&path).map_err(failed)?;
                ledger.add(folder, &text)?;
            }
        }
        Ok(ledger)
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.folders.values().flatten()
    }

    /// Whether a parity run decides the run: it is required, or one of the slice's set.
    pub(crate) fn decides(&self, scenario: &str, run: &str) -> bool {
        self.entries()
            .filter(|entry| entry.scenario == scenario && entry.run == run)
            .any(|entry| entry.gate.is_some() || entry.status == Status::Required)
    }

    /// Everything wrong with the ledger against the runs the runner plans, one line each.
    pub(crate) fn problems(&self, planned: &[Planned]) -> Vec<String> {
        let mut folders: BTreeMap<&str, Vec<&Planned>> = BTreeMap::new();
        for run in planned {
            folders.entry(&run.folder).or_default().push(run);
        }
        let mut problems = Vec::new();
        for (folder, runs) in &folders {
            match self.folders.get(*folder) {
                Some(entries) => problems.extend(
                    folder_problems(entries, runs)
                        .into_iter()
                        .map(|problem| format!("{folder}.toml: {problem}")),
                ),
                None => problems.push(format!(
                    "{folder}.toml is missing: tests/qa/scenarios/{folder} has no ledger"
                )),
            }
        }
        let strays = self
            .folders
            .keys()
            .filter(|folder| !folders.contains_key(folder.as_str()));
        problems.extend(
            strays.map(|folder| format!("{folder}.toml names no folder of tests/qa/scenarios")),
        );
        problems
    }
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

/// Everything wrong with one folder's file against the folder's runs, which are in the
/// runner's order.
fn folder_problems(entries: &[Entry], planned: &[&Planned]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut listed = BTreeSet::new();
    let mut previous = None;
    for entry in entries {
        let name = format!("{}/{}", entry.scenario, entry.run);
        let place = planned
            .iter()
            .position(|run| run.scenario == entry.scenario && run.run == entry.run);
        if !listed.insert((entry.scenario.as_str(), entry.run.as_str())) {
            problems.push(format!("{name} is listed twice"));
        }
        let found = entry
            .misplaced(place.map(|place| planned[place]))
            .or_else(|| entry.unowned())
            .or_else(|| entry.unexplained());
        problems.extend(found.map(|problem| format!("{name} {problem}")));
        problems.extend(disordered(&name, place, &mut previous));
    }
    let unlisted = planned
        .iter()
        .filter(|run| !listed.contains(&(run.scenario.as_str(), run.run.as_str())));
    problems.extend(unlisted.map(|run| format!("{}/{} is not listed", run.scenario, run.run)));
    problems
}

/// What is wrong with an entry that follows one the runner plans after it. `previous` is
/// the last entry that named a run, and where the runner plans it; a file kept in the
/// runner's order gives a new entry one place to go.
fn disordered(
    name: &str,
    place: Option<usize>,
    previous: &mut Option<(usize, String)>,
) -> Option<String> {
    let place = place?;
    let (before, earlier) = previous.replace((place, name.to_owned()))?;
    (place < before)
        .then(|| format!("{name} is out of the runner's order: it belongs before {earlier}"))
}

/// Whether the plan has a package of that id.
fn is_package(id: &str) -> bool {
    let planned = |(letter, numbers): &(char, RangeInclusive<u8>)| {
        numbers
            .clone()
            .any(|number| id == format!("{letter}{number}"))
    };
    TRACKS.iter().any(planned)
}

fn said(text: Option<&str>) -> bool {
    text.is_some_and(|text| !text.trim().is_empty())
}

impl Entry {
    /// What is wrong with where the entry stands: `run` is the run it names, if the folder
    /// has it. No such run, or a gate that is not the run's.
    fn misplaced(&self, run: Option<&Planned>) -> Option<String> {
        match (run.map(|run| run.gate), self.gate) {
            (None, _) => Some("is no run of the folder".to_owned()),
            (Some(planned), listed) if planned == listed => None,
            (Some(Some(gate)), _) => Some(format!("belongs to gate {gate:?}")),
            (Some(None), _) => {
                Some("has a gate, and is no Columns run of the slice's set".to_owned())
            }
        }
    }

    /// What is wrong with the owner: no package of the plan, or one of the slice's on a
    /// run it did not take to `required`. So a slice package's entry cannot be demoted.
    fn unowned(&self) -> Option<String> {
        let package = &self.package;
        let slice = SLICE.iter().find(|(id, _)| id == package);
        match slice {
            Some((_, gate)) if !self.required_under(*gate) => Some(format!(
                "names {package:?}, which owns only the required runs of gate {gate:?}"
            )),
            None if !is_package(package) => Some(format!(
                "names {package:?}, which is no package of the parity plan"
            )),
            _ => None,
        }
    }

    fn required_under(&self, gate: Gate) -> bool {
        self.status == Status::Required && self.gate == Some(gate)
    }

    /// Each status carries its own fields and no other's.
    fn unexplained(&self) -> Option<String> {
        let problem = match self.status {
            Status::Required => self.required_without(),
            Status::Pending => self.pending_without(),
            Status::Excluded => self.excluded_without(),
        };
        problem.map(str::to_owned)
    }

    /// Whether the entry says why its run is pending, as only a pending one does.
    fn blocked(&self) -> bool {
        self.reason.is_some() || self.blocker.is_some()
    }

    fn required_without(&self) -> Option<&'static str> {
        if self.host.is_none() {
            Some("is required without a host")
        } else if self.blocked() || self.ruling.is_some() {
            Some("is required, so it has no reason, blocker or ruling")
        } else {
            None
        }
    }

    /// A pending entry may name the host its run will need; nothing asks it to.
    fn pending_without(&self) -> Option<&'static str> {
        if self.reason.is_none() {
            Some("is pending without a reason")
        } else if !said(self.blocker.as_deref()) {
            Some("is pending without a blocker")
        } else if self.ruling.is_some() {
            Some("is pending, so it has no ruling")
        } else {
            None
        }
    }

    fn excluded_without(&self) -> Option<&'static str> {
        if !said(self.ruling.as_deref()) {
            Some("is excluded without a ruling")
        } else if self.host.is_some() || self.blocked() {
            Some("is excluded, so it has no host, reason or blocker")
        } else {
            None
        }
    }
}

/// The gate of a run: G1's one run, or its folder's when it is a Columns run of the set.
fn gate(folder: &str, scenario: &str, run: &Run) -> Option<Gate> {
    if run.mode != COLUMNS {
        return None;
    }
    if (scenario, run.label.as_str()) == FIRST_LIGHT {
        return Some(Gate::G1);
    }
    SET.iter()
        .find(|(name, _)| *name == folder)
        .map(|(_, gate)| *gate)
}

/// Every run of every scenario file, as the runner plans them.
pub(crate) fn planned() -> Vec<Planned> {
    let mut planned = Vec::new();
    for path in scenario_files() {
        let folder = path.parent().map(file_name).unwrap_or_default();
        let scenario =
            load_scenario(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let name = &scenario.header.scenario;
        for run in runs(&scenario.header) {
            planned.push(Planned {
                folder: folder.to_owned(),
                scenario: name.clone(),
                gate: gate(folder, name, &run),
                run: run.label,
            });
        }
    }
    planned
}
