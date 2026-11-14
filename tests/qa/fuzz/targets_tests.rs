// SPDX-License-Identifier: MIT

//! The differential mode: which targets a command takes, what of their replays is compared
//! with what, and what a run on two targets says and keeps apart.

use super::{
    observe_tests::stream,
    options::Options,
    replay::Stream,
    report::{Examined, Run},
    rules::{Entry, Ignored},
    targets::{Left, Targets, comparisons},
};

fn parse(line: &str) -> Result<Options, String> {
    Options::parse(line.split_whitespace().map(str::to_owned))
}

fn on(target: &str, focused: &[&str]) -> Stream {
    let mut stream = stream(focused);
    stream.header.target = target.to_owned();
    stream
}

fn entry(field: &str, code: &str) -> Entry {
    Entry {
        field: field.to_owned(),
        code: code.to_owned(),
        issue: "lgse/strata#1".to_owned(),
        only: None,
    }
}

pub(crate) fn a_command_takes_one_target_or_gtk_and_rune_and_the_comparing_commands_take_both() {
    let differential = parse("run --alphabet calibration --seeds 1 --target gtk,rune")
        .expect("a run takes both targets");
    assert!(differential.targets.is_differential());
    assert_eq!(differential.targets.label(), "gtk-rune");
    let processors = std::thread::available_parallelism().map_or(1, usize::from);
    assert_eq!(differential.jobs, (processors / 2).max(1));
    for line in [
        "replay --target gtk,rune",
        "shrink --alphabet calibration --seeds 1 --target gtk,rune",
        "run --alphabet calibration --seeds 1 --target rune",
        "run --alphabet calibration --seeds 1 --target gtk,rune --jobs 128",
    ] {
        assert!(parse(line).is_ok(), "{line}");
    }
    let one = parse("run --alphabet calibration --seeds 1 --target rune").expect("one target");
    assert_eq!(
        (one.targets.label(), one.targets.is_differential()),
        ("rune".to_owned(), false)
    );
    let calibration = "run --alphabet calibration --seeds 1";
    for (line, expected) in [
        (
            format!("{calibration} --target rune,gtk"),
            "names GTK first",
        ),
        (
            format!("{calibration} --target qt"),
            "gtk, rune or gtk,rune",
        ),
        (
            format!("{calibration} --target gtk,rune --jobs 129"),
            "at most 256",
        ),
        (
            "generate --alphabet calibration --seeds 1 --target gtk,rune".to_owned(),
            "for run, replay and shrink",
        ),
        (
            "check --target gtk,rune".to_owned(),
            "for run, replay and shrink",
        ),
    ] {
        let error = parse(&line).expect_err(expected);
        assert!(
            error.contains(expected),
            "{error:?} should say {expected:?}"
        );
    }
}

pub(crate) fn each_target_is_held_to_itself_and_rune_to_the_first_gtk_replay_under_the_map() {
    let targets = Targets::parse("gtk,rune").expect("both targets");
    let none = Ignored::default();
    let streams = [
        on("gtk", &["archive", "documents"]),
        on("gtk", &["archive", "documents"]),
        on("rune", &["archive", "pictures"]),
        on("rune", &["archive", "pictures"]),
    ];
    let found = comparisons(
        &streams,
        &targets,
        &Left {
            same: &none,
            across: &none,
        },
    );
    let between: Vec<&str> = found.iter().map(|one| one.between.as_str()).collect();
    assert_eq!(between, ["on gtk", "on rune", "gtk against rune"]);
    assert!(found[0].divergence.is_none() && found[1].divergence.is_none());
    let across = found[2].divergence.as_ref().expect("the targets differ");
    assert_eq!((across.step, across.field()), (1, "app.focused"));
    assert_eq!(
        found[2].compared,
        2 * 2,
        "two Rune replays, two observations each"
    );

    // A rule of the map holds between targets and never within one; a mask holds in both.
    let fields = ["app.focused", "app.panes"];
    let rule = Ignored {
        rules: fields.map(|field| entry(field, "deviation")).to_vec(),
        masks: Vec::new(),
    };
    let mask = Ignored {
        rules: Vec::new(),
        masks: fields.map(|field| entry(field, "unsettled")).to_vec(),
    };
    let mut noisy = streams.clone();
    noisy[1] = on("gtk", &["archive", "pictures"]);
    for (same, across, differs) in [
        (&none, &rule, [true, false, false]),
        (&mask, &mask, [false, false, false]),
    ] {
        let found = comparisons(&noisy, &targets, &Left { same, across });
        let differ: Vec<bool> = found.iter().map(|one| one.divergence.is_some()).collect();
        assert_eq!(differ, differs);
    }

    // One target compares only with itself, as calibration does.
    let alone = comparisons(
        &streams[..2],
        &Targets::parse("gtk").expect("one target"),
        &Left {
            same: &none,
            across: &none,
        },
    );
    assert_eq!(alone.len(), 1);
    assert_eq!(alone[0].between, "on gtk");
}

pub(crate) fn a_run_on_two_targets_names_the_replays_that_differ_and_keeps_its_own_folder() {
    let options = parse("run --alphabet calibration --seeds 1 --target gtk,rune")
        .expect("a run takes both targets");
    let none = Ignored::default();
    let sequence = super::generate::generate(
        &super::alphabet::load("calibration").unwrap_or_else(|error| panic!("{error}")),
        1,
        2,
    );
    let mut examined = Examined::new("sample", &sequence, &options.targets);
    examined.streams = vec![
        on("gtk", &["archive", "documents"]),
        on("gtk", &["archive", "documents"]),
        on("rune", &["archive", "pictures"]),
        on("rune", &["archive", "documents"]),
    ];
    examined.comparisons = comparisons(
        &examined.streams,
        &options.targets,
        &Left {
            same: &none,
            across: &none,
        },
    );
    let mut run = Run::new(&options, &none);
    let lines = run.judge(&examined);
    let name = &sequence.name;
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with(&format!("DIFFER {name} on rune at step 1 ")),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with(&format!("DIFFER {name} gtk against rune at step 1 ")),
        "{lines:?}"
    );
    assert!(run.status().is_err());
    let summary = run.summary();
    assert!(
        summary
            .iter()
            .any(|line| line.starts_with("FUZZ steps on rune: 0 refused")),
        "{summary:?}"
    );
    assert!(
        summary
            .iter()
            .any(|line| line == "FUZZ artifacts: target/qa-fuzz/sample/gtk-rune/"),
        "{summary:?}"
    );

    // A difference between the targets is never counted with a target's own, which is
    // what calibration reads to grow the mask.
    examined.streams[3] = on("rune", &["archive", "pictures"]);
    examined.comparisons = comparisons(
        &examined.streams,
        &options.targets,
        &Left {
            same: &none,
            across: &none,
        },
    );
    let mut run = Run::new(&options, &none);
    run.judge(&examined);
    let summary = run.summary();
    let line = |starts: &str| {
        summary
            .iter()
            .find(|line| line.starts_with(starts))
            .unwrap_or_else(|| panic!("{starts}: {summary:?}"))
            .clone()
    };
    assert_eq!(
        line("FUZZ fields that differed first, with nothing masked,"),
        "FUZZ fields that differed first, with nothing masked, by sequence: none"
    );
    assert!(
        line("FUZZ fields that differed first between the targets").contains("app.focused 1"),
        "{summary:?}"
    );
}
