// SPDX-License-Identifier: MIT

//! The generator, the alphabet and sequence loaders and the command's selections, and the
//! tree's own fuzz files loaded as a run loads them.

use serde_json::json;

use super::{
    alphabet,
    corpus::still_written,
    generate::{Origin, generate},
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
    assert_eq!(sequence.name, "fuzz-sample-v1-0000000000000007");
    let origin = Origin {
        area: "sample".to_owned(),
        version: 1,
        seed: 7,
    };
    assert_eq!(Origin::of(&sequence.name), Some(origin));
    assert_eq!(
        sequence.header,
        r#"{"scenario":"fuzz-sample-v1-0000000000000007","modes":["columns"]}"#
    );
    assert_eq!(sequence.steps, SEED_SEVEN);
    // A longer sequence of the seed starts with the shorter one, and another seed differs.
    let longer = generate(&alphabet, 7, 20);
    assert_eq!(longer.steps[..SEED_SEVEN.len()], SEED_SEVEN);
    assert_ne!(generate(&alphabet, 8, 8).steps, sequence.steps);
    assert_eq!(
        generate(&alphabet, u64::MAX, 4).steps[1],
        SEED_SEVEN[2].replace("shift", "ctrl")
    );
    sequence
        .load()
        .expect("a generated sequence should be a scenario the replay can run");
    // An opening comes first, draws nothing, and the shrinker is told how long it is.
    let opened = sample().replace(HEADER, &format!("{HEADER}\n{OPENING}"));
    let opened = alphabet::parse(&opened).expect("an alphabet with an opening should load");
    let started = generate(&opened, 7, 8);
    assert_eq!(
        started.steps[..2],
        [r#"{"key":"Home"}"#, r#"{"key":"End"}"#]
    );
    assert_eq!(started.steps[2..], SEED_SEVEN);
    assert_eq!(opened.opening_of(&started.steps), 2);
    assert_eq!(opened.opening_of(&sequence.steps), 0);
}

/// Two opening steps for the sample.
const OPENING: &str = r#"opening = ['{"key":"Home"}', '{"key":"End"}']"#;

/// An opening step chooses nothing; a portal's request needs its bus, and a sequence's answer
/// waits, checking nothing: the observation holds what the portal answered.
pub(crate) fn an_opening_and_a_portal_request_are_refused_for_what_a_replay_could_not_run() {
    let sample = sample();
    let portal = sample.replace(
        HEADER,
        r#"header = '{"modes":["columns"],"portal":true,"session_bus":true}'"#,
    );
    for (text, expected) in [
        (
            sample.replace(
                HEADER,
                &format!("{HEADER}\nopening = ['{{\"key\":\"<key>\"}}']"),
            ),
            "an opening step is one line of JSON with nothing to choose",
        ),
        (
            sample.replace(HEADER, r#"header = '{"modes":["columns"],"portal":true}'"#),
            "session_bus",
        ),
        (
            portal.replace(
                r#"'{"key":"Down"}'"#,
                r#"'{"portal_response":{"done":true,"status":0}}'"#,
            ),
            "only waits, so it takes no status",
        ),
    ] {
        let error = alphabet::parse(&text).expect_err(expected);
        assert!(
            error.contains(expected),
            "{error:?} should say {expected:?}"
        );
    }
    alphabet::parse(&portal.replace(
        r#"'{"key":"Down"}'"#,
        r#"'{"portal_open":"Pick"}', '{"portal_response":{"done":true}}'"#,
    ))
    .expect("a portal alphabet that sends a request and waits for its answer should load");
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
            with_header(r#"{"modes":["columns","list"]}"#),
            "one run in one mode",
        ),
        (
            with_header(r#"{"modes":["columns"],"targets":["rune"]}"#),
            "is no twin",
        ),
        (with_step(r#"{"key":"Down","as":"moved"}"#), "takes no as"),
        // The scenario loader, which every sequence goes through, refuses a drag source that
        // would run without the run's bus.
        (
            with_step(r#"{"foreign":{"offers":[]},"as":"foreign"}"#),
            "foreign need the header's session_bus",
        ),
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
        name: "fuzz-sample-v1-0000000000000001".to_owned(),
        header: r#"{"scenario":"fuzz-sample-v1-0000000000000001","modes":["columns"]}"#.to_owned(),
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
        // A drag source in another process, the runner's own (`peers.rs`): the step that starts
        // it names it, and no other step names anything.
        (
            json!({"foreign": {"offers": []}, "as": "foreign", "when": {"directory": "a"}}),
            true,
        ),
        (json!({"foreign_label": "Foreign drag finished"}), true),
        (json!({"foreign_stop": true}), true),
        (json!({"foreign_label": "done", "as": "label"}), false),
        (
            json!({"drag": {"from": {"kept": "foreign"}, "to": {"entry": "a"}}}),
            true,
        ),
        (json!({"http": "aGk=", "as": "url"}), false),
    ] {
        assert_eq!(check_step(&step).is_ok(), accepted, "{step}");
    }
    // A sequence may ask for a private session bus, which the replay starts.
    let with_bus = Sequence {
        name: "fuzz-sample-v1-0000000000000002".to_owned(),
        header: r#"{"scenario":"fuzz-sample-v1-0000000000000002","modes":["columns"],"session_bus":true}"#
            .to_owned(),
        steps: vec![r#"{"key":"Down"}"#.to_owned()],
    };
    assert!(with_bus.load().is_ok(), "a sequence takes session_bus");
}

pub(crate) fn a_sequence_s_name_says_the_alphabet_version_and_the_seed_it_came_from() {
    let origin = Origin {
        area: "columns-pilot".to_owned(),
        version: 12,
        seed: u64::MAX,
    };
    assert_eq!(origin.name(), "fuzz-columns-pilot-v12-ffffffffffffffff");
    assert_eq!(Origin::of(&origin.name()), Some(origin));
    for name in [
        // A seed alone names no sequence: another version writes another one.
        "fuzz-sample-0000000000000007",
        "fuzz-sample-v0-0000000000000007",
        "fuzz-sample-v1-7",
        "fuzz-sample-v1-000000000000000G",
        "fuzz-sample-v01-0000000000000007",
        "sample-v1-0000000000000007",
        "fuzz--v1-0000000000000007",
    ] {
        assert_eq!(Origin::of(name), None, "{name}");
    }
}

pub(crate) fn an_alphabet_s_version_is_pinned_to_what_its_seeds_write() {
    let sample = sample();
    let alphabet = alphabet::parse(&sample).expect("the sample alphabet should load");
    let pins = format!("other v4 0000000000000000\n{}\n", alphabet.pin());
    assert_eq!(alphabet.check_pin(&pins), Ok(()));
    // What a seed does not depend on is not pinned.
    let reworded = sample.replace("three actions", "3 actions");
    let reworded = alphabet::parse(&reworded).expect("the alphabet should load");
    assert_eq!(reworded.check_pin(&pins), Ok(()));
    // Nor is the offscreen host, the default; the session host is, so that an alphabet that
    // moves there takes a new version and its parked sequences stay with the host they met.
    let offscreen = sample.replace(HEADER, &format!("{HEADER}\nhost = \"offscreen\""));
    let offscreen = alphabet::parse(&offscreen).expect("the alphabet should load");
    assert_eq!(offscreen.check_pin(&pins), Ok(()));
    // A change to what seeds write, without a new version, is refused; so is a new
    // version nobody pinned, and an alphabet with no line at all.
    for (changed, expected) in [
        (
            sample.replace("weight = 3", "weight = 4"),
            "raise `version`",
        ),
        (sample.replace("\"todo.txt\", ", ""), "raise `version`"),
        (
            sample.replace("\"key\":\"Escape\"", "\"key\":\"Up\""),
            "raise `version`",
        ),
        (sample.replace("version = 1", "version = 2"), "does not pin"),
        (
            sample.replace(HEADER, &format!("{HEADER}\nhost = \"session\"")),
            "raise `version`",
        ),
        (
            sample.replace(HEADER, &format!("{HEADER}\n{OPENING}")),
            "raise `version`",
        ),
    ] {
        let changed = alphabet::parse(&changed).expect("the alphabet should load");
        let error = changed.check_pin(&pins).expect_err(expected);
        assert!(
            error.contains(expected),
            "{error:?} should say {expected:?}"
        );
    }
    assert!(alphabet.check_pin("").is_err());
}

pub(crate) fn a_kept_file_of_the_alphabet_s_version_is_made_of_steps_its_seed_writes() {
    let alphabet = alphabet::parse(&sample()).expect("the sample alphabet should load");
    let written = generate(&alphabet, 7, 8);
    // A sequence cut short or shrunk is steps of the seed's, in their order.
    let shrunk: Vec<String> = [0, 3, 6, 11]
        .iter()
        .map(|index| SEED_SEVEN[*index].to_owned())
        .collect();
    assert!(still_written(&alphabet, 7, &written.with_steps(shrunk)));
    assert!(still_written(&alphabet, 7, &written.with_steps(Vec::new())));
    // One from another alphabet or another seed is not, whatever its name says.
    let other = vec![
        SEED_SEVEN[0].to_owned(),
        r#"{"key":"Down","when":{"focused_pane":true}}"#.to_owned(),
    ];
    assert!(!still_written(&alphabet, 7, &written.with_steps(other)));
}
