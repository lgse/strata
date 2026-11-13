// SPDX-License-Identifier: MIT

//! What a command takes, which sequences it means, and when it fails: the exit status of a
//! run rests on these.

use serde_json::json;

use super::{
    alphabet,
    command::{checked, planned},
    compare::Divergence,
    corpus::{Parked, parked_as, parse_parked},
    generate::generate,
    observe::Outcome,
    observe_tests::{observation, stream},
    options::{Command, Options, seeds},
    project::Difference,
    report::{Examined, Run},
    rules::Ignored,
    targets::Comparison,
};

fn parse(line: &str) -> Result<Options, String> {
    Options::parse(line.split_whitespace().map(str::to_owned))
}

pub(crate) fn a_fuzz_command_names_what_it_needs_and_refuses_the_rest() {
    assert_eq!(seeds("7"), Ok(vec![7]));
    assert_eq!(seeds("3..6"), Ok(vec![3, 4, 5, 6]));
    assert_eq!(seeds("3,9,0x2a"), Ok(vec![3, 9, 42]));
    assert!(seeds("6..3").is_err() && seeds("seven").is_err());
    let run = parse(
        "run --alphabet calibration --seeds 1..3 --length 12 --repeats 3 --jobs 2 --rest-samples 6",
    )
    .expect("the selections should parse");
    assert_eq!(
        (run.command, run.alphabet.as_deref(), run.seeds.len()),
        (Command::Run, Some("calibration"), 3)
    );
    assert_eq!(
        (run.length, run.repeats, run.jobs, run.rest.samples),
        (12, 3, 2, 6)
    );
    for line in [
        "replay",
        "check",
        "replay --parked --alphabet calibration --repeats 24",
        "shrink tests/qa/fuzz/corpus/a/fuzz-a-v1-0000000000000001.jsonl",
        "run --alphabet calibration --seeds 1 --jobs 256",
    ] {
        assert!(parse(line).is_ok(), "{line}");
    }
    let calibration = "run --alphabet calibration --seeds 1";
    for (line, expected) in [
        ("run --seeds 1".to_owned(), "--alphabet and --seeds"),
        ("run --alphabet calibration".to_owned(), "--alphabet and --seeds"),
        (format!("{calibration} --repeats 1"), "--repeats 2 or more"),
        (format!("{calibration} some.jsonl"), "takes no file"),
        (format!("{calibration} --length 0"), "positive number"),
        (format!("{calibration} --fast"), "no option --fast"),
        (format!("{calibration} --jobs 257"), "at most 256"),
        (format!("{calibration} --parked"), "replay's survey"),
        ("replay --parked some.jsonl".to_owned(), "replay's survey"),
        (
            "shrink --alphabet calibration --seeds 1..2".to_owned(),
            "one seed or one file",
        ),
        ("scenario".to_owned(), "one sequence file"),
        ("compare one.jsonl".to_owned(), "two stream files"),
        ("fuzz".to_owned(), "no fuzz command"),
        (
            r#"shrink --alphabet calibration --seeds 1 --corpus --perturb {"after":{"key":"a"},"field":"app.focused","value":1}"#.to_owned(),
            "never joins the corpus",
        ),
    ] {
        let error = parse(&line).expect_err(expected);
        assert!(
            error.contains(expected),
            "{error:?} should say {expected:?}"
        );
    }
}

pub(crate) fn the_tree_s_alphabets_map_mask_and_corpus_load_as_a_run_loads_them() {
    let lines = checked().unwrap_or_else(|error| panic!("{error}"));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("FUZZ alphabet calibration ")),
        "the calibration alphabet is what the rest rule was fixed on: {lines:?}"
    );
    for area in alphabet::areas() {
        let alphabet = alphabet::load(&area).unwrap_or_else(|error| panic!("{error}"));
        for seed in 1..=3 {
            generate(&alphabet, seed, 60)
                .load()
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }
}

pub(crate) fn a_parked_line_names_a_version_and_a_seed_and_is_sorted_by_both() {
    let text = "# kept\nv1-000000000000002a oracle lgse/strata#9\n\
                v1-00000000000000ff divergence lgse/strata#10\n\
                v3-000000000000002a oracle lgse/strata#11\n";
    let parked = parse_parked(text).expect("the lines should parse");
    let keys: Vec<(u32, u64)> = parked.iter().map(|one| (one.version, one.seed)).collect();
    assert_eq!(keys, [(1, 42), (1, 255), (3, 42)]);
    // A seed is parked on the version it was found on, and on no other.
    assert_eq!(
        parked_as(&parked, 3, 42).map(|one| one.issue.as_str()),
        Some("lgse/strata#11")
    );
    assert_eq!(parked_as(&parked, 2, 42), None);
    assert_eq!(parked_as(&parked, 3, 255), None);
    for text in [
        "000000000000002a oracle lgse/strata#9\n",
        "v1-2a oracle lgse/strata#9\n",
        "v0-000000000000002a oracle lgse/strata#9\n",
        "v1-000000000000002a noise lgse/strata#9\n",
        "v1-000000000000002a oracle #9\n",
        "v2-000000000000002a oracle lgse/strata#1\nv1-00000000000000ff oracle lgse/strata#2\n",
        "v1-000000000000002a oracle lgse/strata#1\nv1-000000000000002a oracle lgse/strata#2\n",
    ] {
        assert!(parse_parked(text).is_err(), "{text}");
    }
}

pub(crate) fn a_run_skips_a_seed_only_where_it_would_write_what_was_parked() {
    // The tree's own corpus: three of its sequences were found on earlier versions of the
    // calibration alphabet, where their seeds wrote other steps than they write today.
    let alphabet = alphabet::load("calibration").unwrap_or_else(|error| panic!("{error}"));
    let run = parse("run --alphabet calibration --seeds 0x67,0xb1,0x19b,0x41c")
        .expect("the selections should parse");
    let plan = planned(&run).unwrap_or_else(|error| panic!("{error}"));
    let version = alphabet.version;
    let replayed: Vec<&str> = plan
        .jobs
        .iter()
        .map(|job| job.sequence.name.as_str())
        .collect();
    assert_eq!(
        replayed,
        [
            format!("fuzz-calibration-v{version}-0000000000000067"),
            format!("fuzz-calibration-v{version}-00000000000000b1"),
            format!("fuzz-calibration-v{version}-000000000000019b"),
        ]
    );
    let skipped: Vec<&str> = plan.skipped.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        skipped,
        [format!("fuzz-calibration-v{version}-000000000000041c")]
    );
    // A replay of the corpus leaves every parked file out, whatever version it is of, and
    // the survey replays those and no other.
    let replay =
        planned(&parse("replay").expect("replay parses")).unwrap_or_else(|error| panic!("{error}"));
    let survey = planned(&parse("replay --parked").expect("the survey parses"))
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(replay.jobs.iter().all(|job| job.parked.is_none()));
    assert!(survey.skipped.is_empty() && survey.jobs.len() == replay.skipped.len());
    assert!(survey.jobs.iter().all(|job| job.parked.is_some()));
    assert!(
        survey.jobs.len() >= 4,
        "the four sequences calibration parked"
    );
}

fn examined(streams: &[&[&str]]) -> Examined {
    Examined {
        area: "sample".to_owned(),
        name: "fuzz-sample-v1-0000000000000007".to_owned(),
        steps: 2,
        targets: "gtk".to_owned(),
        streams: streams.iter().map(|focused| stream(focused)).collect(),
        comparisons: Vec::new(),
        error: None,
        parked: None,
        kept: None,
    }
}

/// The one comparison of a run on GTK alone.
fn on_gtk(found: Option<Divergence>, unmasked: Option<Divergence>) -> Vec<Comparison> {
    vec![Comparison {
        between: "on gtk".to_owned(),
        across: false,
        divergence: found,
        unmasked,
        compared: 2,
    }]
}

fn differing() -> Divergence {
    Divergence {
        step: 1,
        action: json!({ "key": "Down" }),
        fields: vec![Difference {
            field: "app.focused".to_owned(),
            left: json!("documents"),
            right: json!("pictures"),
        }],
    }
}

fn run_of(line: &str) -> Run {
    let options = parse(line).expect("the selections should parse");
    Run::new(&options, &Ignored::default())
}

pub(crate) fn a_command_fails_on_what_did_not_agree_and_on_nothing_replayed() {
    let agreeing = || examined(&[&["archive", "documents"], &["archive", "documents"]]);
    let mut differs = agreeing();
    differs.comparisons = on_gtk(Some(differing()), Some(differing()));
    let mut masked = agreeing();
    masked.comparisons = on_gtk(None, Some(differing()));
    let mut unsettled = agreeing();
    unsettled.streams[1].observations[1].settled = false;
    let mut warned = agreeing();
    warned.streams[0].observations[1].value["criticals"] = json!(["Gtk-CRITICAL **: no"]);
    let mut lost = agreeing();
    lost.streams[1].observations[1] = observation(1, Outcome::Crashed, "documents", &[]);
    lost.streams[1].gone = Some("strata exited (signal: 11)".to_owned());
    let mut unrun = agreeing();
    unrun.error = Some("no free X display".to_owned());
    for (sequence, starts, fails) in [
        (agreeing(), "AGREE ", false),
        (masked, "MASKED ", false),
        (differs, "DIFFER ", true),
        (unsettled, "UNSETTLED ", true),
        (warned, "FINDING ", true),
        (lost, "FINDING ", true),
        (unrun, "ERROR ", true),
    ] {
        let mut run = run_of("run --alphabet calibration --seeds 1");
        let lines = run.judge(&sequence);
        assert!(lines[0].starts_with(starts), "{lines:?}");
        assert_eq!(run.status().is_err(), fails, "{lines:?}");
        assert!(run.summary()[0].starts_with("FUZZ result: 1 sequences"));
    }
    // A selection that replayed nothing has shown nothing, as a QA selection that ran no
    // scenario has.
    let nothing = run_of("replay").status().expect_err("nothing was replayed");
    assert!(nothing.contains("replayed no sequence"), "{nothing}");
}

pub(crate) fn a_perturbed_run_and_a_survey_of_parked_sequences_say_what_they_are() {
    let perturbation = r#"{"after":{"key":"Down"},"field":"app.focused","value":"perturbed"}"#;
    let mut run = run_of(&format!(
        "run --alphabet calibration --seeds 1 --perturb {perturbation}"
    ));
    let mut differs = examined(&[&["archive", "documents"], &["archive", "perturbed"]]);
    differs.comparisons = on_gtk(Some(differing()), Some(differing()));
    let lines = run.judge(&differs);
    assert!(lines[0].starts_with("PERTURBED DIFFER "), "{lines:?}");
    let failure = run
        .status()
        .expect_err("a perturbed difference fails the run");
    assert!(failure.ends_with("(perturbed on purpose)"), "{failure}");
    assert!(
        run.summary()
            .iter()
            .any(|line| line.starts_with("FUZZ PERTURBED"))
    );
    // A survey reports what a parked sequence does now, and fails only when it cannot run.
    let parked = Parked {
        version: 1,
        seed: 7,
        code: "oracle".to_owned(),
        issue: "lgse/strata#9".to_owned(),
    };
    let mut survey = run_of("replay --parked");
    let mut still = examined(&[&["archive", "documents"], &["archive", "pictures"]]);
    still.parked = Some(parked.clone());
    still.comparisons = on_gtk(Some(differing()), Some(differing()));
    let mut agrees = examined(&[&["archive"], &["archive"], &["archive"]]);
    agrees.parked = Some(parked.clone());
    let lines = [survey.judge(&still), survey.judge(&agrees)].concat();
    assert!(
        lines[0].starts_with("STILL DIFFERS, PARKED (oracle lgse/strata#9): DIFFER "),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("AGREES IN 3 REPLAYS, STILL PARKED (oracle lgse/strata#9): "),
        "{lines:?}"
    );
    assert_eq!(survey.status(), Ok(()));
    let mut unrun = examined(&[]);
    unrun.parked = Some(parked);
    unrun.error = Some("no free X display".to_owned());
    survey.judge(&unrun);
    assert!(survey.status().is_err());
}

pub(crate) fn the_summary_says_how_long_the_target_went_on_changing() {
    let mut run = run_of("run --alphabet calibration --seeds 1");
    assert!(run.summary()[3].starts_with("FUZZ late changes: none in 0 observations"));
    let mut sequence = examined(&[&["archive", "documents", "pictures"]]);
    sequence.streams[0].observations[1].changes = vec![60, 110];
    sequence.streams[0].observations[2].changes = vec![200];
    run.judge(&sequence);
    assert_eq!(
        run.summary()[3],
        "FUZZ late changes: 2 of 3 observations changed after their step's reply; quiet \
         before a late change: median 60 ms, 99th percentile 200 ms, longest 200 ms"
    );
}
