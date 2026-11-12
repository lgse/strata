// SPDX-License-Identifier: MIT

use crate::{
    allowances::{Allowances, Suite, document},
    parity::Ledger,
    parity_tests::{KEYS, PENDING, RENAMES, REQUIRED, Row},
};

/// The allowances are the two the slice was given, and each stands on something real: a
/// run of the suite or a patch a scenario names, a reason, a document that names it and
/// the issue that tracks it.
pub(crate) fn the_ledger_allows_only_the_flaky_run_and_the_stale_patch_it_was_given() {
    let ledger = Ledger::load().unwrap_or_else(|error| panic!("{error}"));
    let allowances = Allowances::load().unwrap_or_else(|error| panic!("{error}"));
    let problems = allowances.problems(&ledger, &Suite::planned(), document);
    assert!(
        problems.is_empty(),
        "tests/qa/parity/slice.toml:\n  {}",
        problems.join("\n  ")
    );
    let flaky: Vec<String> = allowances.flaky.iter().map(|flaky| flaky.name()).collect();
    assert_eq!(
        flaky,
        ["new-folder-with-selection-groups/columns"],
        "a flaky run is allowed by an owner ruling, recorded in docs/rune/slice/README.md"
    );
    let stale: Vec<&str> = allowances
        .stale_patches
        .iter()
        .map(|stale| stale.patch.as_str())
        .collect();
    assert_eq!(stale, ["cut"], "one patch was stale before the slice");
}

/// A ledger of one folder, `keys`, whose runs the allowance cases name.
fn decided() -> Ledger {
    let mut ledger = Ledger::default();
    let opens = |run: &'static str| Row {
        scenario: "opens",
        run,
        ..RENAMES
    };
    let rows = [
        KEYS.entry(PENDING),
        opens("columns").entry(PENDING),
        opens("list").entry(PENDING),
        opens("icons-passing").entry(REQUIRED),
    ];
    ledger
        .add("keys", &rows.concat())
        .unwrap_or_else(|error| panic!("{error}"));
    ledger
}

const FLAKY: &str = "[[flaky]]\nscenario = \"opens\"\nrun = \"columns\"";
const STALE: &str = "[[stale_patch]]\npatch = \"cut\"";
const FOUNDED: &str =
    "reason = \"since before the slice\"\nrecorded = \"notes.md\"\nissue = \"lgse/strata#1\"";

fn allowance_problems(allowances: &str) -> Vec<String> {
    let suite = Suite {
        runs: [
            "moves/columns-keys",
            "opens/columns",
            "opens/list",
            "opens/icons-passing",
        ]
        .map(str::to_owned)
        .into(),
        mutations: ["cut", "paste"].map(str::to_owned).into(),
    };
    let notes = |path: &str| {
        (path == "notes.md").then(|| "opens/columns is flaky and cut.patch is stale".to_owned())
    };
    Allowances::parse(allowances)
        .unwrap_or_else(|error| panic!("{error}"))
        .problems(&decided(), &suite, notes)
}

pub(crate) fn an_allowance_for_nothing_real_or_with_nothing_behind_it_is_refused() {
    let both = format!("{FLAKY}\n{FOUNDED}\n{STALE}\n{FOUNDED}\n");
    assert_eq!(allowance_problems(&both), Vec::<String>::new());
    assert_eq!(allowance_problems(""), Vec::<String>::new());

    let flaky = |scenario: &str, run: &str| {
        format!("[[flaky]]\nscenario = \"{scenario}\"\nrun = \"{run}\"\n{FOUNDED}\n")
    };
    let elsewhere = FOUNDED.replace("notes.md", "README.md");
    let unexplained = FOUNDED.replace("since before the slice", " ");
    let untracked = FOUNDED.replace("lgse/strata#1", "");
    for (case, allowances, expected) in [
        (
            "the run does not exist",
            flaky("opens", "icons"),
            "opens/icons is no run of the GTK suite",
        ),
        (
            "the run is the slice's set's",
            flaky("moves", "columns-keys"),
            "moves/columns-keys is required or in the slice's set, so a parity run decides it",
        ),
        (
            "the run is required",
            flaky("opens", "icons-passing"),
            "opens/icons-passing is required or in the slice's set, so a parity run decides it",
        ),
        (
            "the run exists and no document names it",
            flaky("opens", "list"),
            "opens/list is not recorded in \"notes.md\"",
        ),
        (
            "the document is not there",
            format!("{FLAKY}\n{elsewhere}\n"),
            "opens/columns is not recorded in \"README.md\"",
        ),
        (
            "the run is allowed without a reason",
            format!("{FLAKY}\n{unexplained}\n"),
            "opens/columns is allowed without a reason",
        ),
        (
            "the run is allowed without an issue",
            format!("{FLAKY}\n{untracked}\n"),
            "opens/columns is allowed without an issue",
        ),
        (
            "the run is allowed twice",
            format!("{FLAKY}\n{FOUNDED}\n{FLAKY}\n{FOUNDED}\n"),
            "opens/columns is allowed twice",
        ),
        (
            "no scenario names the patch",
            format!("[[stale_patch]]\npatch = \"trash\"\n{FOUNDED}\n"),
            "trash.patch is no mutation patch a scenario names",
        ),
        (
            "the patch exists and no document names it",
            format!("[[stale_patch]]\npatch = \"paste\"\n{FOUNDED}\n"),
            "paste.patch is not recorded in \"notes.md\"",
        ),
    ] {
        assert_eq!(allowance_problems(&allowances), [expected], "{case}");
    }
}

pub(crate) fn an_allowance_with_a_missing_or_an_unknown_field_does_not_parse() {
    for malformed in [
        format!("{FLAKY}\nreason = \"x\"\nissue = \"x\"\n"),
        format!("{FLAKY}\nreason = \"x\"\nrecorded = \"notes.md\"\n"),
        format!("{FLAKY}\n{FOUNDED}\nuntil = \"V10\"\n"),
        format!("{STALE}\nrecorded = \"notes.md\"\nissue = \"x\"\n"),
        KEYS.entry(PENDING),
    ] {
        assert!(Allowances::parse(&malformed).is_err(), "{malformed}");
    }
}
