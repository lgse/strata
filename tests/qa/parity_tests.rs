// SPDX-License-Identifier: MIT

use crate::{
    parity::{Gate, Ledger, Planned, Status, planned},
    target::{RUNE_BINARY_VARIABLE, TARGET_VARIABLE, Target},
};

/// The folder whose runs an owner ruling left out of the migration: they run on GTK only.
const EXCLUDED: &str = "visual_baselines";

pub(crate) fn the_parity_ledger_accounts_for_every_run_of_every_scenario_folder() {
    let ledger = Ledger::load().unwrap_or_else(|error| panic!("{error}"));
    let planned = planned();
    let problems = ledger.problems(&planned);
    assert!(
        problems.is_empty(),
        "tests/qa/parity:\n  {}",
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

pub(crate) const PENDING: &str = "status = \"pending\"\nreason = \"scope\"\nblocker = \"A7\"";
pub(crate) const REQUIRED: &str = "status = \"required\"\nhost = \"offscreen\"";
const EXCLUDING: &str = "status = \"excluded\"\nruling = \"the owner, 2026-11-11\"";

/// A run of the ledger these tests make up: where the runner plans it, and who owns it.
#[derive(Clone, Copy)]
pub(crate) struct Row {
    pub(crate) folder: &'static str,
    pub(crate) scenario: &'static str,
    pub(crate) run: &'static str,
    pub(crate) package: &'static str,
    pub(crate) gate: Option<Gate>,
}

/// Folder `keys` has three runs, two of them of the slice's set; folder `pictures` has one.
const ARROWS: Row = Row {
    folder: "keys",
    scenario: "moves",
    run: "columns-arrows",
    package: "V7",
    gate: Some(Gate::G1),
};
pub(crate) const KEYS: Row = Row {
    run: "columns-keys",
    package: "A7",
    gate: Some(Gate::G2),
    ..ARROWS
};
pub(crate) const RENAMES: Row = Row {
    scenario: "renames",
    run: "list",
    gate: None,
    ..KEYS
};
const LOOKS: Row = Row {
    folder: "pictures",
    scenario: "looks",
    run: "icons",
    package: "E1",
    gate: None,
};

impl Row {
    fn planned(self) -> Planned {
        Planned {
            folder: self.folder.to_owned(),
            scenario: self.scenario.to_owned(),
            run: self.run.to_owned(),
            gate: self.gate,
        }
    }

    /// The run's entry as a ledger file writes it, ending with the lines of its status.
    pub(crate) fn entry(self, status: &str) -> String {
        let Self {
            scenario,
            run,
            package,
            ..
        } = self;
        let gate = self
            .gate
            .map(|gate| format!("gate = \"{gate:?}\"\n"))
            .unwrap_or_default();
        format!(
            "[[run]]\nscenario = \"{scenario}\"\nrun = \"{run}\"\npackage = \"{package}\"\n\
             {gate}{status}\n"
        )
    }
}

/// The file of folder `keys`, in order but for its last entry, `renames/list`, which is
/// `last`.
fn keys_ending(last: &str) -> String {
    let first = ARROWS.entry(REQUIRED);
    let second = KEYS.entry(PENDING);
    format!("{first}{second}{last}")
}

/// The problems of the ledger of `keys` and `pictures`, given their files.
fn problems(files: &[(&str, &str)]) -> Vec<String> {
    let planned = [ARROWS, KEYS, RENAMES, LOOKS].map(Row::planned);
    let mut ledger = Ledger::default();
    for (folder, text) in files {
        ledger
            .add(folder, text)
            .unwrap_or_else(|error| panic!("{error}"));
    }
    ledger.problems(&planned)
}

/// The problems of the two folders when `keys`' file ends with `last`.
fn problems_ending(last: &str) -> Vec<String> {
    let pictures = LOOKS.entry(EXCLUDING);
    problems(&[("keys", &keys_ending(last)), ("pictures", &pictures)])
}

pub(crate) fn a_ledger_that_misses_repeats_or_misplaces_a_run_is_refused() {
    let third = RENAMES.entry(PENDING);
    let pictures = LOOKS.entry(EXCLUDING);
    assert_eq!(problems_ending(&third), Vec::<String>::new());

    let twice = KEYS.entry(PENDING);
    let gone = Row {
        run: "icons",
        gate: None,
        ..KEYS
    };
    let elsewhere = LOOKS.entry(PENDING);
    let ungated = keys_ending(&third).replace("gate = \"G2\"\n", "");
    let gated = Row {
        gate: Some(Gate::G3),
        ..RENAMES
    };
    let stranger = Row {
        package: "A22",
        ..RENAMES
    };
    for (case, found, expected) in [
        (
            "a run of the folder is missing",
            problems_ending(""),
            "keys.toml: renames/list is not listed",
        ),
        (
            "a run is listed twice",
            problems_ending(&format!("{twice}{third}")),
            "keys.toml: moves/columns-keys is listed twice",
        ),
        (
            "a run no longer exists",
            problems_ending(&format!("{third}{}", gone.entry(PENDING))),
            "keys.toml: moves/icons is no run of the folder",
        ),
        (
            "a run is another folder's",
            problems_ending(&format!("{third}{elsewhere}")),
            "keys.toml: looks/icons is no run of the folder",
        ),
        (
            "a run of the set has no gate",
            problems(&[("keys", &ungated), ("pictures", &pictures)]),
            "keys.toml: moves/columns-keys belongs to gate G2",
        ),
        (
            "a run outside the set has a gate",
            problems_ending(&gated.entry(PENDING)),
            "keys.toml: renames/list has a gate, and is no Columns run of the slice's set",
        ),
        (
            "a run names no package of the plan",
            problems_ending(&stranger.entry(PENDING)),
            "keys.toml: renames/list names \"A22\", which is no package of the parity plan",
        ),
    ] {
        assert_eq!(found, [expected], "{case}");
    }
}

pub(crate) fn a_folder_without_a_ledger_file_or_a_file_without_a_folder_is_refused() {
    let keys = keys_ending(&RENAMES.entry(PENDING));
    let pictures = LOOKS.entry(EXCLUDING);
    assert_eq!(
        problems(&[("keys", &keys)]),
        ["pictures.toml is missing: tests/qa/scenarios/pictures has no ledger"]
    );
    assert_eq!(
        problems(&[("keys", &keys), ("pictures", &pictures), ("sounds", "")]),
        ["sounds.toml names no folder of tests/qa/scenarios"]
    );
}

pub(crate) fn a_ledger_entry_that_lacks_a_field_of_its_status_or_has_anothers_is_refused() {
    let renames = |status: &str| problems_ending(&RENAMES.entry(status));
    for (case, status, expected) in [
        (
            "a pending run gives no reason",
            "status = \"pending\"\nblocker = \"x\"".to_owned(),
            "is pending without a reason",
        ),
        (
            "a pending run names no blocker",
            "status = \"pending\"\nreason = \"rune\"\nblocker = \" \"".to_owned(),
            "is pending without a blocker",
        ),
        (
            "a pending run cites a ruling",
            format!("{PENDING}\nruling = \"x\""),
            "is pending, so it has no ruling",
        ),
        (
            "a run that needs a real window names no session host",
            "status = \"pending\"\nreason = \"platform\"\nblocker = \"x\"".to_owned(),
            "is pending on platform, so its host is session",
        ),
        (
            "a required run names no host",
            "status = \"required\"".to_owned(),
            "is required without a host",
        ),
        (
            "a required run keeps its old excuse",
            format!("{REQUIRED}\nreason = \"harness\"\nblocker = \"x\""),
            "is required, so it has no reason, blocker or ruling",
        ),
        (
            "an excluded run cites no ruling",
            "status = \"excluded\"\nruling = \"\"".to_owned(),
            "is excluded without a ruling",
        ),
        (
            "an excluded run is also blocked",
            format!("{EXCLUDING}\nreason = \"scope\"\nblocker = \"A7\""),
            "is excluded, so it has no host, reason or blocker",
        ),
    ] {
        let expected = format!("keys.toml: renames/list {expected}");
        assert_eq!(renames(&status), [expected], "{case}");
    }
    let on_session =
        "status = \"pending\"\nhost = \"session\"\nreason = \"platform\"\nblocker = \"x\"";
    assert_eq!(renames(on_session), Vec::<String>::new());
}

pub(crate) fn a_ledger_entry_with_an_unknown_value_or_field_or_no_package_does_not_parse() {
    let parse = |text: &str| Ledger::default().add("keys", text);
    for status in [
        "status = \"done\"",
        "status = \"pending\"\nreason = \"flaky\"\nblocker = \"x\"",
        "status = \"required\"\nhost = \"window\"",
        "status = \"required\"\nhost = \"offscreen\"\nnote = \"x\"",
    ] {
        let malformed = ARROWS.entry(status);
        assert!(parse(&malformed).is_err(), "{malformed}");
    }
    let unknown_gate = ARROWS.entry(REQUIRED).replace("\"G1\"", "\"G4\"");
    assert!(parse(&unknown_gate).is_err(), "{unknown_gate}");
    let unowned = ARROWS.entry(REQUIRED).replace("package = \"V7\"\n", "");
    assert!(parse(&unowned).is_err(), "{unowned}");
    let allowance = "[[flaky]]\nscenario = \"moves\"\nrun = \"columns-arrows\"\n";
    assert!(parse(allowance).is_err(), "{allowance}");
    for reason in [
        "harness",
        "component",
        "rune",
        "platform",
        "toolkit",
        "deviation",
        "scope",
    ] {
        let coded = format!("status = \"pending\"\nreason = \"{reason}\"\nblocker = \"x\"");
        assert!(parse(&ARROWS.entry(&coded)).is_ok(), "{coded}");
    }
    for status in [REQUIRED, PENDING, EXCLUDING] {
        assert!(parse(&RENAMES.entry(status)).is_ok(), "{status}");
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
