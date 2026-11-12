// SPDX-License-Identifier: MIT

use crate::{
    parity::{Gate, Ledger, Planned, Status, Suite, document, planned},
    target::{RUNE_BINARY_VARIABLE, TARGET_VARIABLE, Target},
};

pub(crate) fn the_parity_ledger_accounts_for_every_columns_run_of_the_slices_set() {
    let ledger = Ledger::load().unwrap_or_else(|error| panic!("{error}"));
    let planned = planned();
    let problems = ledger.problems(&planned);
    assert!(
        problems.is_empty(),
        "tests/qa/parity/slice.toml:\n  {}",
        problems.join("\n  ")
    );
    let gate = |gate: Gate| planned.iter().filter(|run| run.2 == gate).count();
    assert_eq!(gate(Gate::G1), 1, "gate G1 is one run");
    assert!(gate(Gate::G2) > 0 && gate(Gate::G3) > 0);
    let first_light = ledger.runs.iter().find(|entry| entry.gate == Gate::G1);
    assert_eq!(
        first_light.map(|entry| entry.status),
        Some(Status::Required),
        "gate G1's run is required"
    );
}

/// The allowances are the two the slice was given, and each stands on something real: a
/// run of the suite or a patch a scenario names, a reason, and a document that names it.
pub(crate) fn the_ledger_allows_only_the_flaky_run_and_the_stale_patch_it_was_given() {
    let ledger = Ledger::load().unwrap_or_else(|error| panic!("{error}"));
    let problems = ledger.allowance_problems(&planned(), &Suite::planned(), document);
    assert!(
        problems.is_empty(),
        "tests/qa/parity/slice.toml:\n  {}",
        problems.join("\n  ")
    );
    let flaky: Vec<String> = ledger.flaky.iter().map(|flaky| flaky.name()).collect();
    assert_eq!(
        flaky,
        ["new-folder-with-selection-groups/columns"],
        "a flaky run is allowed by an owner ruling, recorded in docs/rune/slice/README.md"
    );
    let stale: Vec<&str> = ledger
        .stale_patches
        .iter()
        .map(|stale| stale.patch.as_str())
        .collect();
    assert_eq!(stale, ["cut"], "one patch was stale before the slice");
}

fn run(scenario: &str, label: &str, gate: Gate) -> Planned {
    (scenario.to_owned(), label.to_owned(), gate)
}

const PENDING: &str = "status = \"pending\"\nreason = \"harness\"\nblocker = \"open_menu\"";
const REQUIRED: &str = "status = \"required\"";

fn entry(scenario: &str, label: &str, gate: &str, rest: &str) -> String {
    format!("[[run]]\nscenario = \"{scenario}\"\nrun = \"{label}\"\ngate = \"{gate}\"\n{rest}\n")
}

/// A ledger that is in order but for its last entry, `renames/columns`, which is `last`.
fn ledger_ending(last: &str) -> String {
    let first = entry("moves", "columns-arrows", "G1", REQUIRED);
    let second = entry("moves", "columns-keys", "G2", PENDING);
    format!("{first}{second}{last}")
}

fn problems(ledger: &str) -> Vec<String> {
    let planned = [
        run("moves", "columns-arrows", Gate::G1),
        run("moves", "columns-keys", Gate::G2),
        run("renames", "columns", Gate::G3),
    ];
    Ledger::parse(ledger)
        .unwrap_or_else(|error| panic!("{error}"))
        .problems(&planned)
}

pub(crate) fn a_ledger_that_misses_repeats_or_leaves_a_run_unexplained_is_refused() {
    let third = entry("renames", "columns", "G3", PENDING);
    let renames = |rest: &str| ledger_ending(&entry("renames", "columns", "G3", rest));
    assert_eq!(problems(&ledger_ending(&third)), Vec::<String>::new());

    let twice = entry("moves", "columns-keys", "G2", PENDING);
    let gone = entry("moves", "icons", "G2", PENDING);
    for (case, ledger, expected) in [
        (
            "a run of the set is missing",
            ledger_ending(""),
            "renames/columns is not listed",
        ),
        (
            "a run is listed twice",
            ledger_ending(&format!("{twice}{third}")),
            "moves/columns-keys is listed twice",
        ),
        (
            "a run no longer exists",
            ledger_ending(&format!("{third}{gone}")),
            "moves/icons is no Columns run of the set",
        ),
        (
            "a run sits under another gate",
            ledger_ending(&entry("renames", "columns", "G2", PENDING)),
            "renames/columns belongs to gate G3",
        ),
        (
            "a pending run gives no reason",
            renames("status = \"pending\"\nblocker = \"x\""),
            "renames/columns is pending without a reason",
        ),
        (
            "a pending run names no blocker",
            renames("status = \"pending\"\nreason = \"rune\"\nblocker = \" \""),
            "renames/columns is pending without a blocker",
        ),
        (
            "a required run keeps its old excuse",
            renames("status = \"required\"\nreason = \"harness\"\nblocker = \"x\""),
            "renames/columns is required, so it has no reason or blocker",
        ),
    ] {
        assert_eq!(problems(&ledger), [expected], "{case}");
    }
}

const FLAKY: &str = "[[flaky]]\nscenario = \"opens\"\nrun = \"columns\"";
const STALE: &str = "[[stale_patch]]\npatch = \"cut\"";
const FOUNDED: &str = "reason = \"since before the slice\"\nrecorded = \"notes.md\"";

fn allowance_problems(ledger: &str) -> Vec<String> {
    let planned = [run("moves", "columns-keys", Gate::G2)];
    let suite = Suite {
        runs: ["moves/columns-keys", "opens/columns", "opens/list"]
            .map(str::to_owned)
            .into(),
        mutations: ["cut", "paste"].map(str::to_owned).into(),
    };
    let notes = |path: &str| {
        (path == "notes.md").then(|| "opens/columns is flaky and cut.patch is stale".to_owned())
    };
    Ledger::parse(ledger)
        .unwrap_or_else(|error| panic!("{error}"))
        .allowance_problems(&planned, &suite, notes)
}

pub(crate) fn an_allowance_for_nothing_real_or_with_nothing_behind_it_is_refused() {
    let both = format!("{FLAKY}\n{FOUNDED}\n{STALE}\n{FOUNDED}\n");
    assert_eq!(allowance_problems(&both), Vec::<String>::new());
    assert_eq!(allowance_problems(""), Vec::<String>::new());

    let flaky = |scenario: &str, run: &str| {
        format!("[[flaky]]\nscenario = \"{scenario}\"\nrun = \"{run}\"\n{FOUNDED}\n")
    };
    let elsewhere = "reason = \"since before the slice\"\nrecorded = \"README.md\"";
    let unexplained = "reason = \" \"\nrecorded = \"notes.md\"";
    for (case, ledger, expected) in [
        (
            "the run does not exist",
            flaky("opens", "icons"),
            "opens/icons is no run of the GTK suite",
        ),
        (
            "the run is the parity set's",
            flaky("moves", "columns-keys"),
            "moves/columns-keys is a run of the parity set, which a gate decides",
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
        assert_eq!(allowance_problems(&ledger), [expected], "{case}");
    }

    for malformed in [
        format!("{FLAKY}\nreason = \"x\"\n"),
        format!("{FLAKY}\n{FOUNDED}\nuntil = \"V10\"\n"),
        format!("{STALE}\nrecorded = \"notes.md\"\n"),
    ] {
        assert!(Ledger::parse(&malformed).is_err(), "{malformed}");
    }
}

pub(crate) fn a_ledger_entry_with_an_unknown_gate_status_reason_or_field_does_not_parse() {
    for rest in [
        "status = \"done\"",
        "status = \"pending\"\nreason = \"flaky\"\nblocker = \"x\"",
        "status = \"required\"\nnote = \"x\"",
    ] {
        let malformed = entry("moves", "columns-arrows", "G1", rest);
        assert!(Ledger::parse(&malformed).is_err(), "{malformed}");
    }
    let unknown_gate = entry("moves", "columns-arrows", "G4", REQUIRED);
    assert!(Ledger::parse(&unknown_gate).is_err(), "{unknown_gate}");
    for reason in [
        "harness",
        "component",
        "rune",
        "platform",
        "toolkit",
        "deviation",
        "scope",
    ] {
        let rest = format!("status = \"pending\"\nreason = \"{reason}\"\nblocker = \"x\"");
        let coded = entry("moves", "columns-arrows", "G1", &rest);
        assert!(Ledger::parse(&coded).is_ok(), "{coded}");
    }
}

pub(crate) fn the_target_picks_the_binary_and_only_rune_is_told_the_layout() {
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
}
