// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    REPOSITORY, harness,
    parity::Ledger,
    parity_tests::Edit,
    scenario::{load_scenario, plan},
    target::Target,
    twins::Corpus,
};

/// What both of the ledger's readers accept and refuse of a labelled pair. The scripts'
/// tests read the same table for theirs (`scripts/test_qa_parity_twins.py`).
const CASES: &str = "tests/qa/parity_pair_cases.toml";

/// The table's corpus: scenario files with a labelled pair, their ledger, the Rune
/// mutation patches and the packet's entries.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pairs {
    patches: Vec<String>,
    packet: Vec<String>,
    /// A file's text for each `<folder>/<scenario>`.
    scenarios: BTreeMap<String, String>,
    ledger: BTreeMap<String, String>,
    #[serde(rename = "case")]
    refused: Vec<PairCase>,
    accepted: Vec<PairCase>,
}

/// Edits to the corpus, and what a reader makes of the result.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairCase {
    name: String,
    #[serde(rename = "edit", default)]
    edits: Vec<Edit>,
    /// The patches there are, when the case changes them.
    patches: Option<Vec<String>>,
    /// The one problem the readers report, word for word.
    problem: Option<String>,
    /// What a reader that cannot read a file says, in its own words.
    refused: Option<String>,
}

impl Pairs {
    fn load() -> Self {
        let text = fs::read_to_string(Path::new(REPOSITORY).join(CASES))
            .unwrap_or_else(|error| panic!("{CASES}: {error}"));
        toml::from_str(&text).unwrap_or_else(|error| panic!("{CASES}: {error}"))
    }

    /// What the reader makes of the corpus after a case's edits: its problems, where a
    /// ledger file it cannot read is one.
    fn outcome(&self, case: Option<&PairCase>) -> Vec<String> {
        let (mut scenarios, mut files) = (self.scenarios.clone(), self.ledger.clone());
        let name = case.map(|case| case.name.as_str()).unwrap_or_default();
        for edit in case.iter().flat_map(|case| &case.edits) {
            let of_header = edit.header.is_some();
            edit.apply(
                name,
                if of_header {
                    &mut scenarios
                } else {
                    &mut files
                },
            );
        }
        let patches = case.and_then(|case| case.patches.as_ref());
        let mut corpus = Corpus::citing(patches.unwrap_or(&self.patches), &self.packet);
        for (path, text) in &scenarios {
            let (folder, scenario) = path.split_once('/').expect("<folder>/<scenario>");
            corpus.add(folder, scenario, text);
        }
        let mut ledger = Ledger::default();
        let read = files
            .iter()
            .try_for_each(|(folder, text)| ledger.add(folder, text));
        read.map_or_else(|error| vec![error], |()| ledger.faults(&corpus))
    }
}

pub(crate) fn the_pair_checks_accept_and_refuse_what_the_shared_table_says() {
    let pairs = Pairs::load();
    assert_eq!(
        pairs.outcome(None),
        Vec::<String>::new(),
        "the table's pair"
    );
    for case in &pairs.accepted {
        let outcome = pairs.outcome(Some(case));
        assert_eq!(outcome, Vec::<String>::new(), "{}", case.name);
    }
    for case in &pairs.refused {
        let outcome = pairs.outcome(Some(case));
        match (&case.problem, &case.refused) {
            (Some(problem), None) => assert_eq!(outcome, [problem.as_str()], "{}", case.name),
            (None, Some(refused)) => {
                assert_eq!(outcome.len(), 1, "{}: {outcome:?}", case.name);
                assert!(
                    outcome[0].contains(refused.as_str()),
                    "{}: {outcome:?}",
                    case.name
                );
            }
            _ => panic!("{}: a case has a problem or is refused", case.name),
        }
    }
}

/// A scenario file with one step, by its name and the rest of its header.
fn written(directory: &Path, name: &str, rest: &str) -> PathBuf {
    let path = directory.join(format!("{name}.jsonl"));
    let text = format!("{{\"scenario\":\"{name}\",{rest}}}\n{{\"key\":\"Escape\"}}\n");
    fs::write(&path, text).expect("the scenario should be written");
    path
}

const LABEL: &str = r#""exception":"timing","ref":"lgse/strata#2200""#;

/// `moves-rune`, the twin of `moves` in Columns, with what its header says of its targets.
fn twin(directory: &Path, targets: &str) -> PathBuf {
    let parity = format!(r#""parity":{{"twin":"moves",{LABEL},"differs":"line 2"}}"#);
    let rest = format!(r#""modes":["columns"],{targets}{parity}"#);
    written(directory, "moves-rune", &rest)
}

/// The twin and its original, `moves`, in the runner's order.
fn pair(directory: &Path) -> [PathBuf; 2] {
    let label = format!(r#""parity":{{"twin":"moves-rune",{LABEL}}}"#);
    let original = written(
        directory,
        "moves",
        &format!(r#""modes":["columns","list"],{label}"#),
    );
    [twin(directory, r#""targets":["rune"],"#), original]
}

/// A twin lists Rune as its one target, so on GTK it is no run at all: the suite plans,
/// counts and names what it did before the twin was written.
pub(crate) fn a_twin_is_planned_on_rune_alone_and_leaves_the_gtk_suite_as_it_was() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let files = pair(directory.path());
    for file in &files {
        load_scenario(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    }

    let planned = |files: &[PathBuf], target: Option<Target>| {
        let plan = plan(files, target, false);
        let runs = plan.tests.iter().filter(|test| test.group.is_some());
        let names: Vec<String> = runs.map(|test| test.name.clone()).collect();
        (names, plan.tests.len(), plan.notice(target))
    };
    let (alone, counted, notice) = planned(&files[1..], Some(Target::Gtk));
    assert_eq!(alone, ["moves/columns", "moves/list"]);
    assert_eq!(notice, None);
    let (beside, counted_beside, notice) = planned(&files, Some(Target::Gtk));
    assert_eq!((beside, counted_beside), (alone, counted));
    let notice = notice.expect("the twin is named as left out");
    assert!(notice.starts_with("NOT ON gtk: 1 scenario(s)"), "{notice}");
    assert!(notice.ends_with(": moves-rune"), "{notice}");

    let (on_rune, _, notice) = planned(&files, Some(Target::Rune));
    assert_eq!(
        on_rune,
        ["moves-rune/columns", "moves/columns", "moves/list"]
    );
    assert_eq!(notice, None);
    // A target the environment cannot name plans every file, and each launch says why.
    assert_eq!(planned(&files, None).0.len(), 3);

    let on_both = twin(directory.path(), "");
    let refused = load_scenario(&on_both)
        .map(|_| ())
        .expect_err("a twin on GTK");
    assert!(
        refused.contains("is a twin that also runs on GTK"),
        "{refused}"
    );
}

/// A scenario named as `./scripts/qa.sh` names one, which is no run on the target, fails
/// the selection whatever else it names; a substring that happens to cover it does not.
pub(crate) fn a_named_scenario_that_is_no_run_on_the_target_fails_the_selection() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let files = pair(directory.path());
    let selection = |arguments: &[&str], target: Target| {
        let arguments = harness::Arguments::parse(arguments.iter().map(|&text| text.to_owned()))
            .expect("the arguments should parse");
        plan(&files, Some(target), false).unrunnable(&arguments, Some(target))
    };
    for named in [
        &["--exact", "moves-rune", "moves"][..],
        &["--exact", "moves", "moves-rune/columns"],
    ] {
        let refusal = selection(named, Target::Gtk).expect("the twin is no run on GTK");
        assert!(
            refusal.starts_with("moves-rune is no run on gtk"),
            "{refusal}"
        );
        assert!(refusal.contains("qa-parity.sh"), "{refusal}");
    }
    assert_eq!(selection(&["--exact", "moves"], Target::Gtk), None);
    assert_eq!(selection(&["moves-rune"], Target::Gtk), None);
    assert_eq!(selection(&[], Target::Gtk), None);
    assert_eq!(selection(&["--exact", "moves-rune"], Target::Rune), None);
}
