// SPDX-License-Identifier: MIT

use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

use crate::{
    REPOSITORY,
    parity::{Gate, HELP, Ledger, Planned, Status},
    target::{RUNE_BINARY_VARIABLE, TARGET_VARIABLE, Target},
    twins::Corpus,
};

/// The folder whose runs an owner ruling left out of the migration: they run on GTK only.
const EXCLUDED: &str = "visual_baselines";

/// Every run of the corpus both targets share is listed once, and every twin once more: as
/// the `twin` of the parallel run it stands in for, with the pair's two labels in order.
pub(crate) fn the_parity_ledger_accounts_for_every_run_of_every_scenario_folder() {
    let ledger = Ledger::load().unwrap_or_else(|error| panic!("{error}"));
    let corpus = Corpus::load();
    let planned = corpus.planned();
    let problems = ledger.faults(&corpus);
    assert!(
        problems.is_empty(),
        "tests/qa/parity and the labelled pairs of tests/qa:\n  {}\n{HELP}",
        problems.join("\n  ")
    );
    let gate = |gate: Gate| planned.iter().filter(|run| run.gate == Some(gate)).count();
    assert_eq!(gate(Gate::G1), 1, "gate G1 is one run");
    assert!(gate(Gate::G2) > 0 && gate(Gate::G3) > 0);
    let first_light = ledger.entries().find(|entry| entry.gate == Some(Gate::G1));
    assert_eq!(
        first_light.map(|entry| entry.status),
        Some(Status::Required),
        "gate G1's run is required"
    );
}

/// An exclusion takes a run out of the migration, so it is an owner ruling and not a place
/// to park a run: the committed ledger excludes one folder, whole.
pub(crate) fn the_ledger_excludes_only_the_runs_the_owner_ruled_out() {
    let ledger = Ledger::load().unwrap_or_else(|error| panic!("{error}"));
    for (folder, entries) in &ledger.folders {
        for entry in entries {
            assert_eq!(
                entry.status == Status::Excluded,
                folder == EXCLUDED,
                "{}/{}: the visual baselines are excluded, by the ruling of 2026-11-11, and \
                 nothing else is",
                entry.scenario,
                entry.run
            );
        }
    }
}

/// What both of the ledger's readers accept and refuse. The scripts' tests read the same
/// table for theirs (`scripts/test_qa_parity.py`), so the two cannot drift apart.
const CASES: &str = "tests/qa/parity_cases.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cases {
    planned: Vec<Planned>,
    ledger: BTreeMap<String, String>,
    #[serde(rename = "case")]
    refused: Vec<Case>,
    accepted: Vec<Case>,
}

/// An edit to the table's ledger, and what a reader makes of the result.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    file: String,
    find: Option<String>,
    put: Option<String>,
    append: Option<String>,
    text: Option<String>,
    #[serde(default)]
    remove: bool,
    /// The one problem the readers report, word for word.
    problem: Option<String>,
    /// What a reader's refusal to read the file says, in its own words.
    refused: Option<String>,
}

/// An edit to one text of a table: a ledger file's, or a scenario file's, which the pair
/// table names by `header`.
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Edit {
    pub(crate) file: Option<String>,
    pub(crate) header: Option<String>,
    find: Option<String>,
    put: Option<String>,
    append: Option<String>,
    text: Option<String>,
    #[serde(default)]
    remove: bool,
}

impl Edit {
    /// Makes the edit to the text it names, among a table's headers or its ledger files.
    pub(crate) fn apply(&self, case: &str, texts: &mut BTreeMap<String, String>) {
        let key = self.header.as_ref().or(self.file.as_ref());
        let key = key.unwrap_or_else(|| panic!("{case}: an edit names a header or a file"));
        let before = texts.remove(key).unwrap_or_default();
        texts.extend(self.edited(case, &before).map(|after| (key.clone(), after)));
    }

    /// The text after the edit; nothing when the edit removes it. `case` names the case.
    pub(crate) fn edited(&self, case: &str, before: &str) -> Option<String> {
        let after = match (&self.text, &self.find) {
            (Some(text), _) => text.clone(),
            (None, Some(find)) => {
                let found = before.matches(find.as_str()).count();
                assert_eq!(found, 1, "{case}: `find` occurs once in the text");
                before.replace(find.as_str(), self.put.as_deref().unwrap_or_default())
            }
            (None, None) => format!("{before}{}", self.append.as_deref().unwrap_or_default()),
        };
        (!self.remove).then_some(after)
    }
}

impl Case {
    /// The file's text after the edit; nothing when the edit removes the file.
    fn edited(&self, before: &str) -> Option<String> {
        let edit = Edit {
            find: self.find.clone(),
            put: self.put.clone(),
            append: self.append.clone(),
            text: self.text.clone(),
            remove: self.remove,
            ..Edit::default()
        };
        edit.edited(&self.name, before)
    }
}

impl Cases {
    fn load() -> Self {
        let text = fs::read_to_string(Path::new(REPOSITORY).join(CASES))
            .unwrap_or_else(|error| panic!("{CASES}: {error}"));
        toml::from_str(&text).unwrap_or_else(|error| panic!("{CASES}: {error}"))
    }

    /// What the reader makes of the table's ledger after an edit: its problems, or why it
    /// could not read a file.
    fn outcome(&self, case: Option<&Case>) -> Result<Vec<String>, String> {
        let mut files = self.ledger.clone();
        if let Some(case) = case {
            let before = files.remove(&case.file).unwrap_or_default();
            files.extend(case.edited(&before).map(|after| (case.file.clone(), after)));
        }
        let mut ledger = Ledger::default();
        for (folder, text) in &files {
            ledger.add(folder, text)?;
        }
        Ok(ledger.problems(&self.planned))
    }
}

pub(crate) fn the_ledger_reader_accepts_and_refuses_what_the_shared_table_says() {
    let cases = Cases::load();
    assert_eq!(
        cases.outcome(None),
        Ok(Vec::new()),
        "the table's own ledger"
    );
    for case in &cases.accepted {
        assert_eq!(cases.outcome(Some(case)), Ok(Vec::new()), "{}", case.name);
    }
    for case in &cases.refused {
        let outcome = cases.outcome(Some(case));
        match (&case.problem, &case.refused) {
            (Some(problem), None) => {
                assert_eq!(outcome, Ok(vec![problem.clone()]), "{}", case.name);
            }
            (None, Some(refused)) => {
                let error = outcome.expect_err(&case.name);
                assert!(error.contains(refused.as_str()), "{}: {error}", case.name);
            }
            _ => panic!("{}: a case has a problem or is refused", case.name),
        }
    }
}

pub(crate) fn the_target_picks_the_binary_and_only_rune_is_told_the_layout_or_asked_its_invariants()
{
    assert_eq!(Target::named(None), Ok(Target::Gtk));
    assert_eq!(Target::named(Some("")), Ok(Target::Gtk));
    assert_eq!(Target::named(Some("gtk")), Ok(Target::Gtk));
    assert_eq!(Target::named(Some("rune")), Ok(Target::Rune));
    let unknown = Target::named(Some("qt")).expect_err("no such target");
    assert!(unknown.contains(TARGET_VARIABLE), "{unknown}");

    // These tests also run from a bundle of test binaries alone, with no application binary
    // on disk. Whether the GTK binary is built is the launch's to find out; its path is
    // cargo's, and absolute, so no installed build is found through PATH instead. The Rune
    // target checks its file itself, so the test makes one to stand for strata-rune.
    let stand_in = tempfile::NamedTempFile::new().expect("a file to stand for strata-rune");
    let rune = stand_in.path().as_os_str();
    let gtk = Target::Gtk.binary(None).expect("the application's binary");
    assert!(gtk.is_absolute(), "{}", gtk.display());
    assert_eq!(gtk.file_name(), Some("strata".as_ref()));
    assert_eq!(Target::Gtk.binary(Some(rune)), Ok(gtk), "GTK takes no path");
    let unset = Target::Rune.binary(None).expect_err("no path");
    assert!(unset.contains(RUNE_BINARY_VARIABLE), "{unset}");
    assert!(Target::Rune.binary(Some("".as_ref())).is_err());
    let missing = Target::Rune
        .binary(Some("/nonexistent/strata-rune".as_ref()))
        .expect_err("no file");
    assert!(missing.contains("/nonexistent/strata-rune"), "{missing}");
    assert_eq!(
        Target::Rune.binary(Some(rune)),
        Ok(stand_in.path().to_owned())
    );
    assert_ne!(Target::Gtk.artifacts(), Target::Rune.artifacts());

    let manifest = crate::fixture::manifest();
    let russian = &manifest.layouts["ru"];
    assert_eq!(Target::Gtk.keymap_step(russian), None);
    let step = Target::Rune.keymap_step(russian).expect("a step");
    assert_eq!(
        step,
        serde_json::json!({ "keymap": {
            "layouts": "us,ru", "variants": "", "group": 1, "caps_lock": false
        } })
    );
    // Every layout of the manifest makes a step the application takes from the runner, and
    // none a scenario could write itself.
    for layout in manifest.layouts.values() {
        let step = Target::Rune
            .keymap_step(layout)
            .expect("a step")
            .to_string();
        assert!(
            matches!(
                crate::protocol::parse_request(&step),
                Ok(crate::protocol::Command::Keymap(_))
            ),
            "{step}"
        );
        assert!(crate::protocol::parse_command(&step).is_err(), "{step}");
    }
    // Only Rune is asked for its invariants, by a step no scenario could write either.
    assert_eq!(Target::Gtk.invariants_step(), None);
    let step = Target::Rune.invariants_step().expect("a step").to_string();
    assert_eq!(
        crate::protocol::parse_request(&step),
        Ok(crate::protocol::Command::Invariants)
    );
    assert!(crate::protocol::parse_command(&step).is_err(), "{step}");
}
