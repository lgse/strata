// SPDX-License-Identifier: MIT

//! The parity ledger's allowances, `tests/qa/parity/slice.toml`: what the GTK suite already
//! did wrong before the slice and `./scripts/qa-parity.sh slice` lets stand, each by an
//! owner ruling (`docs/rune/slice/README.md`). The runs are in the files beside it:
//! `parity.rs`.

use std::{collections::BTreeSet, fs, path::Path};

use serde::Deserialize;

use crate::{
    REPOSITORY,
    parity::Ledger,
    scenario::{load_scenario, scenario_files},
    target::Target,
    variables::runs,
};

pub(crate) const ALLOWANCES: &str = "tests/qa/parity/slice.toml";

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
    /// The `oracle` issue that tracks the flake.
    pub(crate) issue: String,
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
    /// The `oracle` issue that tracks the patch.
    pub(crate) issue: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Allowances {
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
            // A twin runs on Rune alone, so it is no run of the GTK suite.
            if !scenario.header.runs_on(Some(Target::Gtk)) {
                continue;
            }
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

impl Allowances {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|error| format!("{ALLOWANCES}: {error}"))
    }

    pub(crate) fn load() -> Result<Self, String> {
        let path = Path::new(REPOSITORY).join(ALLOWANCES);
        let text = fs::read_to_string(&path).map_err(|error| format!("{ALLOWANCES}: {error}"))?;
        Self::parse(&text)
    }

    /// Everything wrong with the allowances, one line each. `ledger` says which runs a
    /// parity run decides, `suite` is the whole GTK suite and `document` reads what an
    /// allowance cites.
    pub(crate) fn problems(
        &self,
        ledger: &Ledger,
        suite: &Suite,
        document: impl Fn(&str) -> Option<String>,
    ) -> Vec<String> {
        let flaky = self
            .flaky
            .iter()
            .map(|flaky| (flaky.cited(), flaky.misplaced(ledger, suite)));
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
            issue: &self.issue,
        }
    }

    /// What is wrong with the run named: none of the suite's, or one a parity run decides.
    fn misplaced(&self, ledger: &Ledger, suite: &Suite) -> Option<String> {
        if !suite.runs.contains(&self.name()) {
            Some("is no run of the GTK suite".to_owned())
        } else if ledger.decides(&self.scenario, &self.run) {
            Some(
                "is required, parallel or in the slice's set, so a parity run decides it"
                    .to_owned(),
            )
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
            issue: &self.issue,
        }
    }

    fn misplaced(&self, suite: &Suite) -> Option<String> {
        (!suite.mutations.contains(&self.patch))
            .then(|| "is no mutation patch a scenario names".to_owned())
    }
}

/// What every allowance owes: why it stands, a document that names it and its issue.
struct Cited<'a> {
    name: String,
    reason: &'a str,
    recorded: &'a str,
    issue: &'a str,
}

impl Cited<'_> {
    fn unfounded(&self, document: &impl Fn(&str) -> Option<String>) -> Option<String> {
        if self.reason.trim().is_empty() {
            return Some("is allowed without a reason".to_owned());
        }
        if self.issue.trim().is_empty() {
            return Some("is allowed without an issue".to_owned());
        }
        let named = document(self.recorded).is_some_and(|text| text.contains(&self.name));
        (!named).then(|| format!("is not recorded in {:?}", self.recorded))
    }
}
