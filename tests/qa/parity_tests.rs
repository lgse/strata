// SPDX-License-Identifier: MIT

use crate::{
    parity::{Gate, Ledger, Planned, Status, planned},
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

    let gtk = Target::Gtk.binary(None).expect("the application's binary");
    assert!(gtk.is_file(), "{}", gtk.display());
    let unset = Target::Rune.binary(None).expect_err("no path");
    assert!(unset.contains(RUNE_BINARY_VARIABLE), "{unset}");
    assert!(Target::Rune.binary(Some("".as_ref())).is_err());
    let missing = Target::Rune
        .binary(Some("/nonexistent/strata-rune".as_ref()))
        .expect_err("no file");
    assert!(missing.contains("/nonexistent/strata-rune"), "{missing}");
    assert_eq!(Target::Rune.binary(Some(gtk.as_os_str())), Ok(gtk));
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
