// SPDX-License-Identifier: MIT

//! The generator, the alphabet and sequence loaders and the command's selections, and the
//! tree's own fuzz files loaded as a run loads them.

use serde_json::json;

use super::{
    alphabet,
    command::checked,
    generate::{generate, name, seed_of},
    options::{Command, Options, seeds},
    random::Random,
    sequence::{Sequence, check_step},
};

const DOWN: &str = "[[action]]\nname = \"down\"\nweight = 3\nsteps = ['{\"key\":\"Down\"}']\n";

const HEADER: &str = "header = '{\"modes\":[\"columns\"]}'";

/// Three actions for the generator's own test: one plain, one of two steps with two names
/// to choose, one that acts on the fixture.
fn sample() -> String {
    format!(
        r#"area = "sample"
version = 1
about = "three actions for the generator's own test"
{HEADER}

{DOWN}
[[action]]
name = "pick"
weight = 2
steps = ['{{"click":{{"entry":"<entry>"}},"mods":["<modifier>"],"when":{{"present":{{"entry":"<entry>"}}}}}}', '{{"key":"Escape"}}']
choose.entry = ["archive", "todo.txt", "readme.md"]
choose.modifier = ["ctrl", "shift"]

[[action]]
name = "write"
weight = 1
steps = ['{{"fs":{{"path":"<path>","write":"x"}}}}']
choose.path = ["a.txt", "b.txt"]
"#
    )
}

/// What seed 7 writes from the sample, worked out by a second implementation of the
/// generator: SplitMix64, a draw by weight, then one draw for each name in name order.
const SEED_SEVEN: [&str; 12] = [
    r#"{"click":{"entry":"archive"},"mods":["ctrl"],"when":{"present":{"entry":"archive"}}}"#,
    r#"{"key":"Escape"}"#,
    r#"{"click":{"entry":"todo.txt"},"mods":["shift"],"when":{"present":{"entry":"todo.txt"}}}"#,
    r#"{"key":"Escape"}"#,
    r#"{"click":{"entry":"archive"},"mods":["shift"],"when":{"present":{"entry":"archive"}}}"#,
    r#"{"key":"Escape"}"#,
    r#"{"fs":{"path":"b.txt","write":"x"}}"#,
    r#"{"click":{"entry":"archive"},"mods":["ctrl"],"when":{"present":{"entry":"archive"}}}"#,
    r#"{"key":"Escape"}"#,
    r#"{"key":"Down"}"#,
    r#"{"key":"Down"}"#,
    r#"{"key":"Down"}"#,
];

pub(crate) fn a_seed_writes_the_same_sequence_on_every_machine() {
    // SplitMix64's published first outputs for seed 0.
    let mut random = Random::new(0);
    assert_eq!(
        [random.next(), random.next(), random.next()],
        [
            0xE220_A839_7B1D_CDAF,
            0x6E78_9E6A_A1B9_65F4,
            0x06C4_5D18_8009_454F
        ]
    );
    let alphabet = alphabet::parse(&sample()).expect("the sample alphabet should load");
    let sequence = generate(&alphabet, 7, 8);
    assert_eq!(sequence.name, "fuzz-sample-0000000000000007");
    assert_eq!(seed_of(&sequence.name), Some(7));
    assert_eq!(
        sequence.header,
        r#"{"scenario":"fuzz-sample-0000000000000007","modes":["columns"]}"#
    );
    assert_eq!(sequence.steps, SEED_SEVEN);
    // A longer sequence of the seed starts with the shorter one, and another seed differs.
    let longer = generate(&alphabet, 7, 20);
    assert_eq!(longer.steps[..SEED_SEVEN.len()], SEED_SEVEN);
    assert_ne!(generate(&alphabet, 8, 8).steps, sequence.steps);
    assert_eq!(name("sample", u64::MAX), "fuzz-sample-ffffffffffffffff");
    assert_eq!(
        generate(&alphabet, u64::MAX, 4).steps[1],
        SEED_SEVEN[2].replace("shift", "ctrl")
    );
    sequence
        .load()
        .expect("a generated sequence should be a scenario the replay can run");
}

pub(crate) fn an_alphabet_is_refused_for_what_a_replay_could_not_run() {
    let sample = sample();
    let with_step = |step: &str| sample.replace(r#"'{"key":"Down"}'"#, &format!("'{step}'"));
    let with_header = |header: &str| sample.replace(HEADER, &format!("header = '{header}'"));
    let cases = [
        (
            sample.replace("version = 1", "version = 0"),
            "version starts at 1",
        ),
        (
            sample.replace("\"sample\"", "\"Sample\""),
            "lowercase words",
        ),
        (
            sample.replace("weight = 3", "weight = 0"),
            "weight is at least 1",
        ),
        (
            sample.replace("\"pick\"", "\"down\""),
            "two actions are named",
        ),
        (
            sample.replace("choose.path = [\"a.txt\", \"b.txt\"]", ""),
            "<path> with nothing to choose from",
        ),
        (
            sample.replace("<modifier>", "ctrl"),
            "no step uses <modifier>",
        ),
        (
            sample.replace("about = ", "note = \"\"\nabout = "),
            "unknown field",
        ),
        (
            with_header(r#"{"modes":["columns"],"session_bus":true}"#),
            "does not take the header's session_bus",
        ),
        (
            with_header(r#"{"modes":["columns","list"]}"#),
            "one run in one mode",
        ),
        (
            with_header(r#"{"modes":["columns"],"targets":["rune"]}"#),
            "is no twin",
        ),
        (with_step(r#"{"key":"Down","as":"moved"}"#), "takes no as"),
        (with_step(r#"{"expect":{"focused":true}}"#), "only asks"),
        (
            with_step(r#"{"fs":{"path":"a","exists":true}}"#),
            "acts on the fixture",
        ),
        (
            with_step(r#"{"restart":true}"#),
            "process steps and fs actions only",
        ),
        (with_step(r#"{"key":"Down""#), "is not JSON"),
        (
            with_step(r#"{"key":"Down","repeat":"twice"}"#),
            "invalid key command",
        ),
    ];
    for (text, expected) in cases {
        let error = alphabet::parse(&text).expect_err(expected);
        assert!(
            error.contains(expected),
            "{error:?} should say {expected:?}"
        );
    }
}

pub(crate) fn a_sequence_is_scenario_text_and_takes_only_steps_that_act() {
    let sequence = Sequence {
        name: "fuzz-sample-0000000000000001".to_owned(),
        header: r#"{"scenario":"fuzz-sample-0000000000000001","modes":["columns"]}"#.to_owned(),
        steps: vec![r#"{"key":"Down"}"#.to_owned(), r#"{"type":"a"}"#.to_owned()],
    };
    assert_eq!(Sequence::parse(&sequence.text()), Ok(sequence.clone()));
    assert_eq!(sequence.with_steps(Vec::new()).text().lines().count(), 1);
    let loaded = sequence.load().expect("the sequence should load");
    assert_eq!(
        (loaded.run.label.as_str(), loaded.scenario.steps.len()),
        ("columns", 2)
    );
    let renamed = Sequence {
        name: "sample".to_owned(),
        header: r#"{"scenario":"sample","modes":["columns"]}"#.to_owned(),
        steps: Vec::new(),
    };
    assert!(
        renamed.load().is_err(),
        "a sequence is named fuzz-<area>-<seed>"
    );
    for (step, accepted) in [
        (json!({"key": "Down"}), true),
        (
            json!({"click": {"entry": "a"}, "when": {"present": {"entry": "a"}}}),
            true,
        ),
        (json!({"fs": {"path": "a", "write": "x"}}), true),
        (
            json!({"fs": {"path": "a", "remove": true}, "when": {"entry": "a"}}),
            true,
        ),
        (json!({"fs": {"path": "a", "text": "x"}}), false),
        (json!({"wait": {"focused": "a"}, "timeout_ms": 2000}), true),
        (json!({"expect": {"focused": "a"}}), false),
        (json!({"query": "state"}), false),
        (json!({"key": "Down", "if": "held"}), false),
        (json!({"key": "Down", "modes": ["columns"]}), false),
        (json!({"layout": "ru"}), false),
        (json!({"screenshot": "a.png"}), false),
    ] {
        assert_eq!(check_step(&step).is_ok(), accepted, "{step}");
    }
}

pub(crate) fn a_fuzz_command_names_what_it_needs_and_refuses_the_rest() {
    let parse = |line: &str| Options::parse(line.split_whitespace().map(str::to_owned));
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
    assert!(parse("replay").is_ok() && parse("check").is_ok());
    assert!(parse("shrink tests/qa/fuzz/corpus/a/fuzz-a-0000000000000001.jsonl").is_ok());
    for (line, expected) in [
        ("run --seeds 1", "--alphabet and --seeds"),
        ("run --alphabet calibration", "--alphabet and --seeds"),
        (
            "run --alphabet calibration --seeds 1 --repeats 1",
            "--repeats 2 or more",
        ),
        (
            "run --alphabet calibration --seeds 1 some.jsonl",
            "takes no file",
        ),
        (
            "shrink --alphabet calibration --seeds 1..2",
            "one seed or one file",
        ),
        ("scenario", "one sequence file"),
        ("compare one.jsonl", "two stream files"),
        (
            "run --alphabet calibration --seeds 1 --length 0",
            "positive number",
        ),
        (
            "run --alphabet calibration --seeds 1 --fast",
            "no option --fast",
        ),
        ("fuzz", "no fuzz command"),
        (
            r#"shrink --alphabet calibration --seeds 1 --corpus --perturb {"after":{"key":"a"},"field":"app.focused","value":1}"#,
            "never joins the corpus",
        ),
    ] {
        let error = parse(line).expect_err(expected);
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
