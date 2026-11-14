// SPDX-License-Identifier: MIT

//! What an observation holds and when it is taken: the projection, the names of its fields,
//! the rest rule, and a stream's file.

use std::{cell::Cell, time::Duration};

use serde_json::{Value, json};

use super::{
    observe::{Faults, Observation, Outcome, Rest, Rested, Sample, at_rest, digest},
    project::{self, Places, covers, differences, names_a_field, pattern_of, set_at, value_at},
    replay::{Header, Stream, answered, gone},
};

pub(crate) fn the_projection_keeps_the_ten_keys_and_names_the_places_of_a_run() {
    let places = Places::new("/tmp/strata-qa-fixture-ab12CD", "/tmp/strata-qa-xy34");
    let reply = json!({
        "view_mode": "Columns",
        "directory": "strata-qa-fixture-ab12CD",
        "panes": [{"name": "strata-qa-fixture-ab12CD", "selected": ["archive"], "focused": null}],
        "focused": "archive",
        "preview": ["/tmp/strata-qa-fixture-ab12CD/todo.txt", "/tmp/strata-qa-xy34/home/x"],
        "added_later": 1,
    });
    let app = project::app(&reply, &places);
    let keys: Vec<&String> = app.as_object().expect("an object").keys().collect();
    let mut expected = project::APP_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        keys, expected,
        "the ten keys, a missing one null, and no other"
    );
    assert_eq!(app["directory"], "$root");
    assert_eq!(app["panes"][0]["name"], "$root");
    assert_eq!(
        app["preview"],
        json!(["$fixture/todo.txt", "$environment/home/x"])
    );
    assert_eq!(app["dialog"], Value::Null);
    assert_eq!(project::app(&Value::Null, &places), Value::Null);
    // A setting keyed by a path is the same setting in every run.
    assert_eq!(
        places.normalise(&json!({"/tmp/strata-qa-fixture-ab12CD/a": 1})),
        json!({"$fixture/a": 1})
    );
}

pub(crate) fn a_field_is_named_by_its_path_and_an_entry_covers_what_is_below_it() {
    let left = json!({
        "result": "ok",
        "criticals": [],
        "app": {"focused": "a", "matches": ["a", "b"], "field": {"text": "a", "caret": 1},
                "panes": [{"name": "r", "selected": ["a"]}, {"name": "d", "selected": []}]},
        "fs": {"documents/notes.txt": "file 644 6 aa", "todo.txt": "file 644 5 bb"},
        "settings": {"show_hidden": false, "keymap": {"default": {"rename": ["F2"]}}},
    });
    let right = json!({
        "result": "refused",
        "criticals": [],
        "app": {"focused": "b", "matches": ["b", "a"], "field": {"text": "a", "caret": 2},
                "panes": [{"name": "r", "selected": ["a"]}, {"name": "d", "selected": ["x"]}]},
        "fs": {"documents/notes.txt": "file 644 7 cc"},
        "settings": {"show_hidden": true, "keymap": {"default": {"rename": ["F3"]}}},
    });
    let fields: Vec<String> = differences(&left, &right)
        .into_iter()
        .map(|difference| difference.field)
        .collect();
    assert_eq!(
        fields,
        [
            "result",
            "app.field.caret",
            "app.focused",
            "app.matches",
            "app.panes[1].selected",
            "fs.documents/notes.txt",
            "fs.todo.txt",
            "settings.keymap.default.rename",
            "settings.show_hidden",
        ]
    );
    assert!(differences(&left, &left).is_empty());
    // Panes that are not as many are one difference, not a guess at which pane moved.
    let fewer = json!({"app": {"panes": [{"name": "r"}]}});
    let more = json!({"app": {"panes": [{"name": "r"}, {"name": "d"}]}});
    assert_eq!(differences(&fewer, &more)[0].field, "app.panes");
    assert_eq!(pattern_of("app.panes[12].selected"), "app.panes[].selected");
    // A file's name keeps its dots and brackets: only a list of `app` has places.
    assert_eq!(pattern_of("fs.report[1].txt"), "fs.report[1].txt");
}

pub(crate) fn an_entry_covers_its_field_and_what_is_below_it_at_a_boundary() {
    for (entry, field, covered) in [
        ("app.panes[].selected", "app.panes[1].selected", true),
        ("app.panes", "app.panes[1].selected", true),
        ("app.panes", "app.panes", true),
        ("app.field", "app.field.caret", true),
        ("app.focused", "app.focused_pane", false),
        ("app.panes[].selected", "app.panes[0].focused", false),
        ("settings.keymap", "settings.keymap.default.rename", true),
        ("settings.key", "settings.keymap.default.rename", false),
        // A folder covers its files; part of a name covers nothing.
        ("fs.documents", "fs.documents/notes.txt", true),
        ("fs.documents/notes.txt", "fs.documents/notes.txt", true),
        ("fs.documents/notes", "fs.documents/notes.txt", false),
        ("fs.doc", "fs.documents/notes.txt", false),
        ("fs.report[1].txt", "fs.report[1].txt", true),
        ("fs.report[].txt", "fs.report[1].txt", false),
        ("fs.documents", "settings.documents", false),
    ] {
        assert_eq!(covers(entry, field), covered, "{entry} over {field}");
    }
    for (entry, named) in [
        ("app.focused", true),
        ("app.panes[].selected", true),
        ("fs.documents/notes.txt", true),
        ("settings.show_hidden", true),
        ("settings.keymap.default", true),
        // A whole root, the outcome, the warnings and the invariants are never left out.
        ("app", false),
        ("fs", false),
        ("settings", false),
        ("result", false),
        ("criticals", false),
        ("invariants", false),
        ("invariants[]", false),
        ("app.panes[0].selected", false),
        ("app.rows", false),
        ("reply.ok", false),
        ("app.", false),
    ] {
        assert_eq!(names_a_field(entry), named, "{entry}");
    }
    // A perturbation reads and writes a field by the same names, nested settings too.
    let mut observed = json!({
        "app": {"panes": [{"name": "r"}, {"name": "d"}]},
        "fs": {"report[1].txt": "file 644 1 aa"},
        "settings": {"keymap": {"default": {"rename": ["F2"]}}},
    });
    for (field, held) in [
        ("app.panes[1].name", json!("d")),
        ("fs.report[1].txt", json!("file 644 1 aa")),
        ("settings.keymap.default.rename", json!(["F2"])),
    ] {
        assert_eq!(value_at(&observed, field), Some(&held), "{field}");
        assert!(set_at(&mut observed, field, json!("changed")), "{field}");
        assert_eq!(
            value_at(&observed, field),
            Some(&json!("changed")),
            "{field}"
        );
    }
    assert!(!set_at(&mut observed, "app.panes[5].name", json!(1)));
    assert_eq!(value_at(&observed, "app.panes[].name"), None);
}

pub(crate) fn the_projection_names_every_key_the_snapshot_has() {
    // A key added to the snapshot has to be added to the projection, or said to be left
    // out: it must not go uncompared without anyone having decided so.
    let snapshot = serde_json::to_value(crate::protocol::Observation::default().app_state())
        .expect("the snapshot serialises");
    let mut keys: Vec<&str> = snapshot
        .as_object()
        .expect("the snapshot is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut projected = project::APP_KEYS.to_vec();
    projected.sort_unstable();
    assert_eq!(keys, projected);
}

pub(crate) fn a_replay_ends_where_the_application_leaves_the_fixture() {
    let tree = json!({ "documents": "dir 755", "documents/notes.txt": "file 644 6 aa", "todo.txt": "file 644 5 bb" });
    let panes = |names: &[&str]| json!({ "panes": names.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>() });
    for (names, left) in [
        // Columns keeps the fixture's own column first for as long as it is inside.
        (&["$root"][..], false),
        (&["$root", "documents"][..], false),
        // The parent's listing is the machine's, also once the fixture is entered again.
        (&["tmp"][..], true),
        (&["tmp", "$root"][..], true),
        // The other views show one pane: a folder of the fixture, or one that is not.
        (&["documents"][..], false),
        (&["todo.txt"][..], true),
        (&["home"][..], true),
        // No pane, no folder: a chooser before it opens.
        (&[][..], false),
    ] {
        assert_eq!(project::outside(&panes(names), &tree), left, "{names:?}");
    }
    let ended: Vec<bool> = [
        Outcome::Ok,
        Outcome::Skipped,
        Outcome::Refused,
        Outcome::Failed,
        Outcome::Crashed,
        Outcome::Closed,
        Outcome::Left,
    ]
    .into_iter()
    .map(|outcome| {
        let observed = observation(1, outcome, "archive", &[]);
        assert_eq!(observed.ends_the_replay(), outcome.ends_the_replay());
        outcome.ends_the_replay()
    })
    .collect();
    assert_eq!(ended, [false, false, false, false, true, true, true]);
}

pub(crate) fn rest_is_equal_samples_in_a_row_and_the_budget_never_fails_a_step() {
    let sample = |focused: &str| Sample {
        app: json!({ "focused": focused }),
        ..Sample::default()
    };
    let rest = Rest {
        samples: 3,
        poll: Duration::ZERO,
        budget: Duration::from_secs(60),
    };
    // The application is still moving for three reads, then stops.
    let script = ["a", "b", "b", "c", "c", "c", "d"];
    let read = Cell::new(0);
    let (rested, gone) = at_rest(
        &mut || {
            read.set(read.get() + 1);
            Ok(sample(script[read.get() - 1]))
        },
        &rest,
    );
    assert_eq!(
        (rested.sample, rested.settled, rested.polls, gone),
        (sample("c"), true, 6, None)
    );
    assert_eq!(rested.changes.len(), 2, "it changed to b and then to c");
    // One that never stops is given up on when the budget is spent, and is not an error.
    let turn = Cell::new(0);
    let spent = Rest {
        budget: Duration::ZERO,
        ..rest
    };
    let (rested, gone) = at_rest(
        &mut || {
            turn.set(turn.get() + 1);
            Ok(sample(&turn.get().to_string()))
        },
        &spent,
    );
    assert_eq!((rested.settled, rested.polls, gone), (false, 1, None));
    // A target that stops answering ends the wait with the last thing it said.
    let asked = Cell::new(0);
    let (rested, gone) = at_rest(
        &mut || {
            asked.set(asked.get() + 1);
            if asked.get() == 1 {
                Ok(sample("a"))
            } else {
                Err("exited".to_owned())
            }
        },
        &rest,
    );
    assert_eq!(
        (rested.sample, rested.settled, gone),
        (sample("a"), false, Some("exited".to_owned()))
    );
}

pub(crate) fn a_stream_keeps_its_observations_through_its_file() {
    let mut kept = stream(&["archive", "documents"]);
    kept.gone = Some("no reply within 90 s".to_owned());
    kept.observations[1].changes = vec![40, 95];
    assert_eq!(Stream::parse(&kept.text()), Ok(kept.clone()));
    assert_eq!(
        (kept.count(Outcome::Ok), kept.unsettled(), kept.criticals()),
        (2, 0, 0)
    );
    let line = kept.observations[1].line();
    assert_eq!(
        line["fs_digest"],
        digest(&json!({ "todo.txt": "file 644 5 aa" }))
    );
    // A digest is of what a value says, not of the order its keys were written in.
    let reordered: Value =
        serde_json::from_str(r#"{"b":[1,{"d":2,"c":3}],"a":"x"}"#).expect("JSON");
    assert_eq!(
        digest(&reordered),
        digest(&json!({ "a": "x", "b": [1, { "c": 3, "d": 2 }] }))
    );
    assert_ne!(digest(&json!({ "a": 1 })), digest(&json!({ "a": 2 })));
    // A stream without its header is not read, and neither is a line it does not know.
    let text = kept.text();
    let body = text.split_once('\n').expect("a header line").1;
    assert!(Stream::parse(body).is_err());
    assert!(Stream::parse(&format!("{text}{{\"later\":1}}\n")).is_err());
}

pub(crate) fn two_streams_are_compared_only_when_they_answer_one_another() {
    let ours = stream(&["archive"]).header;
    assert_eq!(ours.mismatch(&ours.clone()), None);
    // Another target is what the differential mode compares; the map then applies.
    let rune = Header {
        target: "rune".to_owned(),
        ..ours.clone()
    };
    assert_eq!(ours.mismatch(&rune), None);
    let slower = Rest {
        samples: 40,
        ..ours.rest
    };
    for (other, part) in [
        (
            Header {
                sequence: "fuzz-sample-v1-0000000000000008".to_owned(),
                ..ours.clone()
            },
            "sequence",
        ),
        (
            Header {
                version: 2,
                ..ours.clone()
            },
            "alphabet version",
        ),
        (
            Header {
                rest: slower,
                ..ours.clone()
            },
            "rest rule",
        ),
    ] {
        let refusal = ours.mismatch(&other).expect(part);
        assert!(refusal.ends_with(part), "{refusal}");
    }
}

pub(crate) fn a_step_s_outcome_is_what_the_target_answered_or_how_it_went_away() {
    for (reply, outcome, lost) in [
        (json!({"ok": true, "error": null}), Outcome::Ok, false),
        (
            json!({"ok": false, "error": "no entry named a"}),
            Outcome::Refused,
            false,
        ),
        (
            json!({"ok": false, "error": null, "failures": [{}]}),
            Outcome::Failed,
            false,
        ),
        (
            json!({"ok": false, "error": "the QA controller panicked: boom"}),
            Outcome::Crashed,
            true,
        ),
    ] {
        let (answer, gone) = answered(&reply);
        assert_eq!((answer, gone.is_some()), (outcome, lost), "{reply}");
    }
    // A window that was closed exits with success, and that is the step's outcome; any
    // other exit, and a target that is still there and silent, is a crash with its reason.
    assert_eq!(
        gone(Some(true), "exited".to_owned()),
        (Outcome::Closed, None)
    );
    for exited in [Some(false), None] {
        assert_eq!(
            gone(exited, "no reply within 90 s".to_owned()),
            (Outcome::Crashed, Some("no reply within 90 s".to_owned()))
        );
    }
}

pub(super) fn observation(
    step: usize,
    outcome: Outcome,
    focused: &str,
    selected: &[&str],
) -> Observation {
    let action = if step == 0 {
        Value::Null
    } else {
        json!({ "key": "Down" })
    };
    Observation::new(
        (step, action),
        outcome,
        Faults::default(),
        Rested {
            sample: Sample {
                app: json!({
                    "focused": focused,
                    "panes": [{ "name": "$root", "selected": selected }],
                }),
                fs: json!({ "todo.txt": "file 644 5 aa" }),
                settings: json!({}),
            },
            settled: true,
            polls: 4,
            waited: Duration::from_millis(150),
            changes: Vec::new(),
        },
    )
}

pub(super) fn stream(focused: &[&str]) -> Stream {
    Stream {
        header: Header {
            target: "gtk".to_owned(),
            sequence: "fuzz-sample-v1-0000000000000007".to_owned(),
            alphabet: "sample".to_owned(),
            version: 1,
            rest: Rest::default(),
            perturbed: false,
        },
        observations: focused
            .iter()
            .enumerate()
            .map(|(step, focused)| observation(step, Outcome::Ok, focused, &[focused]))
            .collect(),
        gone: None,
    }
}
