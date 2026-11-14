// SPDX-License-Identifier: MIT

//! The runner's own invariant of one stream: an Undo followed at once by a Redo puts the tree
//! back as it was before the Undo.

use serde_json::{Value, json};

use super::{
    observe::{Observation, Outcome},
    observe_tests::observation,
    stream,
};

/// An observation after `key`, ending as `outcome`, with the fixture tree `tree`.
fn after(key: &str, outcome: Outcome, tree: &Value) -> Observation {
    let mut taken = observation(1, outcome, "a", &["a"]);
    taken.action = json!({ "key": key, "when": { "not": { "field": { "name": "Rename" } } } });
    taken.value["fs"] = tree.clone();
    taken
}

fn broken(observation: &Observation) -> Vec<String> {
    observation.value["invariants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

pub(crate) fn an_undo_then_a_redo_puts_the_tree_back_as_it_was_before_the_undo() {
    let before = json!({ "a.txt": "file 644 1 aa" });
    let undone = json!({});
    let other = json!({ "b.txt": "file 644 1 bb" });
    let ok = Outcome::Ok;
    let cases: [(
        &str,
        (&str, Outcome, &Value),
        (&str, Outcome, &Value),
        &[&str],
    ); 6] = [
        (
            "a redo that restores",
            ("ctrl+z", ok, &undone),
            ("ctrl+y", ok, &before),
            &[],
        ),
        (
            "a redo that does not",
            ("ctrl+z", ok, &undone),
            ("ctrl+shift+z", ok, &other),
            &[
                "undo-redo: the tree after ctrl+shift+z is not the tree before ctrl+z: fs.a.txt differs",
            ],
        ),
        // An Undo that changed nothing in the fixture redoes something else, or nothing.
        (
            "an undo outside the fixture",
            ("ctrl+z", ok, &before),
            ("ctrl+y", ok, &other),
            &[],
        ),
        (
            "a skipped redo",
            ("ctrl+z", ok, &undone),
            ("ctrl+y", Outcome::Skipped, &other),
            &[],
        ),
        (
            "a refused undo",
            ("ctrl+z", Outcome::Refused, &undone),
            ("ctrl+y", ok, &other),
            &[],
        ),
        (
            "no undo before",
            ("Down", ok, &undone),
            ("ctrl+y", ok, &other),
            &[],
        ),
    ];
    for (case, undo, redo, expected) in cases {
        let earlier = after("Down", ok, &before);
        let undone = after(undo.0, undo.1, undo.2);
        let mut now = after(redo.0, redo.1, redo.2);
        stream::check(&[earlier, undone], &mut now);
        assert_eq!(broken(&now), expected, "{case}");
    }
}
