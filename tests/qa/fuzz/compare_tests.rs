// SPDX-License-Identifier: MIT

//! The comparator, the shrinker against a stand-in for two targets, the perturbation, and
//! the map, the mask and the parked seeds.

use std::cell::Cell;

use serde_json::{Value, json};

use super::{
    compare::{Divergence, compare},
    corpus::parse_parked,
    observe::Outcome,
    observe_tests::{observation, stream},
    perturb::Perturbation,
    project::Difference,
    rules::{self, CAUSES, EXCEPTIONS, Entry, Ignored},
    shrink::{CONFIRMATIONS, shrink},
};

fn observed(app: Value) -> Value {
    json!({ "result": "ok", "criticals": [], "app": app, "fs": {}, "settings": {} })
}

fn masks(fields: &[&str]) -> Ignored {
    Ignored {
        rules: Vec::new(),
        masks: fields
            .iter()
            .map(|field| Entry {
                field: (*field).to_owned(),
                code: "unsettled".to_owned(),
                issue: "lgse/strata#2127".to_owned(),
            })
            .collect(),
    }
}

pub(crate) fn the_comparator_reports_the_first_field_outside_the_map_and_the_mask() {
    let left = stream(&["archive", "documents", "pictures", "readme.md"]);
    let right = stream(&["archive", "documents", "todo.txt", "todo.txt"]);
    assert_eq!(compare(&left, &left.clone(), &Ignored::default()), None);
    let found = compare(&left, &right, &Ignored::default()).expect("the streams differ");
    assert_eq!((found.step, found.field()), (2, "app.focused"));
    assert_eq!(found.action, json!({ "key": "Down" }));
    assert_eq!(
        found.fields[0],
        Difference {
            field: "app.focused".to_owned(),
            left: json!("pictures"),
            right: json!("todo.txt"),
        }
    );
    assert_eq!(found.fields[1].field, "app.panes[0].selected");
    assert_eq!(
        found.title("columns"),
        r#"[fuzz columns] app.focused differs after {"key":"Down"}"#
    );
    // A masked field is passed over for the next one, and a wholly masked step for the next.
    let next = compare(&left, &right, &masks(&["app.focused"])).expect("the selection differs");
    assert_eq!(
        (next.step, next.signature()),
        (2, "app.panes[].selected".to_owned())
    );
    assert_eq!(
        compare(&left, &right, &masks(&["app.focused", "app.panes"])),
        None
    );
    // A rule of the map is passed over the same way.
    let ruled = Ignored {
        rules: masks(&["app"]).masks,
        masks: Vec::new(),
    };
    assert_eq!(compare(&left, &right, &ruled), None);
    // A target that is lost differs at the step that lost it, whatever else agreed.
    let mut lost = stream(&["archive", "documents"]);
    lost.gone = Some("strata exited".to_owned());
    let found = compare(&left, &lost, &Ignored::default()).expect("one stream is shorter");
    assert_eq!((found.step, found.field()), (2, "result"));
    assert_eq!(
        (&found.fields[0].left, &found.fields[0].right),
        (&json!("ok"), &Value::Null)
    );
}

pub(crate) fn a_warning_or_a_lost_target_diverges_even_when_both_replays_show_it() {
    let warned = || {
        let mut stream = stream(&["archive", "documents", "pictures"]);
        stream.observations[2].value["criticals"] = json!(["Gtk-CRITICAL **: assertion failed"]);
        stream
    };
    let found = compare(&warned(), &warned(), &Ignored::default()).expect("both warned");
    assert_eq!((found.step, found.field()), (2, "criticals"));
    assert_eq!(
        found.title("columns"),
        r#"[fuzz columns] criticals after {"key":"Down"}"#
    );
    let lost = || {
        let mut stream = stream(&["archive", "documents"]);
        stream.observations[1] = observation(1, Outcome::Crashed, "documents", &[]);
        stream.gone = Some("strata exited".to_owned());
        stream
    };
    let found = compare(&lost(), &lost(), &Ignored::default()).expect("both were lost");
    assert_eq!((found.step, found.field()), (1, "result"));
    // Masking what is observed does not excuse it; only an entry that names it does.
    assert!(compare(&warned(), &warned(), &masks(&["app", "fs", "settings"])).is_some());
    assert_eq!(compare(&warned(), &warned(), &masks(&["criticals"])), None);
}

pub(crate) fn a_step_not_at_rest_is_compared_by_its_outcome_alone() {
    let left = stream(&["archive", "documents", "pictures"]);
    let mut moving = stream(&["archive", "notes.txt", "pictures"]);
    moving.observations[1].settled = false;
    assert_eq!(
        compare(&left, &moving, &Ignored::default()),
        None,
        "nothing is compared while the application is still moving"
    );
    moving.observations[1] = observation(1, Outcome::Refused, "notes.txt", &[]);
    moving.observations[1].settled = false;
    let found = compare(&left, &moving, &Ignored::default()).expect("the outcomes differ");
    assert_eq!(
        (found.step, found.field(), found.fields.len()),
        (1, "result", 1)
    );
}

fn divergence(step: usize, field: &str) -> Divergence {
    Divergence {
        step,
        action: json!({ "key": "c" }),
        fields: vec![Difference {
            field: field.to_owned(),
            left: json!("x"),
            right: json!("y"),
        }],
    }
}

/// A stand-in for two targets: they part on `app.focused` at the step that completes
/// `a … b … c`, and on `app.dialog` at any `z` before that.
fn double(steps: &[String]) -> Option<Divergence> {
    let mut wanted = ["a", "b", "c"].iter().peekable();
    for (index, step) in steps.iter().enumerate() {
        if step == "z" {
            return Some(divergence(index + 1, "app.dialog"));
        }
        if wanted.peek().is_some_and(|next| *next == step) {
            wanted.next();
        }
        if wanted.peek().is_none() {
            return Some(divergence(index + 1, "app.focused"));
        }
    }
    None
}

fn steps(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn without(steps: &[String], dropped: &str) -> Vec<String> {
    steps
        .iter()
        .filter(|step| *step != dropped)
        .cloned()
        .collect()
}

pub(crate) fn the_shrinker_keeps_the_fewest_steps_that_still_diverge_and_confirms_them() {
    let long = steps("x y a x x y b y x x a y c x x y z a b c");
    let found = double(&long).expect("the long sequence diverges");
    assert_eq!((found.step, found.field()), (13, "app.focused"));
    let asked = Cell::new(0);
    let shrunk = shrink(&long, &found, 200, &mut |candidate| {
        asked.set(asked.get() + 1);
        Ok(double(candidate))
    })
    .expect("the oracle never fails");
    assert_eq!(shrunk.steps, steps("a b c"));
    assert_eq!(
        (shrunk.divergence.step, shrunk.divergence.field()),
        (3, "app.focused")
    );
    assert!(shrunk.is_confirmed());
    assert_eq!(
        (shrunk.confirmed, shrunk.asked),
        (CONFIRMATIONS, asked.get())
    );
    // A removal that brings out another divergence is not kept: here taking `b` out lets
    // the `z` behind it show first, on another field.
    let other = steps("a b z c x");
    let shifted = |candidate: &[String]| match candidate.iter().position(|step| step == "z") {
        Some(at) if !candidate[..at].contains(&"b".to_owned()) => {
            Some(divergence(at + 1, "app.dialog"))
        }
        _ => double(&without(candidate, "z")).map(|found| Divergence {
            step: candidate.len().min(found.step + 1),
            ..found
        }),
    };
    let found = shifted(&other).expect("diverges");
    assert_eq!(found.field(), "app.focused");
    let kept = shrink(&other, &found, 200, &mut |candidate| Ok(shifted(candidate)))
        .expect("the oracle never fails");
    assert_eq!(kept.divergence.signature(), "app.focused");
    assert!(kept.steps.contains(&"b".to_owned()) && kept.is_confirmed());
    // The oracle's own failure ends the shrink: it is the harness's, not a verdict.
    let long_found = double(&long).expect("diverges");
    assert!(
        shrink(&long, &long_found, 200, &mut |_| Err(
            "no display".to_owned()
        ))
        .is_err()
    );
    // Out of replays, it stops shrinking and still confirms what it has.
    let limited = shrink(&long, &long_found, 1, &mut |candidate| {
        Ok(double(candidate))
    })
    .expect("the oracle never fails");
    assert!(limited.steps.len() > 3 && limited.is_confirmed());
}

pub(crate) fn a_divergence_that_does_not_repeat_is_not_confirmed() {
    let long = steps("x a y b c");
    let found = double(&long).expect("diverges");
    // Noise: the two replays differ every other time, whatever the steps.
    let asked = Cell::new(0);
    let shrunk = shrink(&long, &found, 200, &mut |candidate| {
        asked.set(asked.get() + 1);
        Ok((asked.get() % 2 == 0 && !candidate.is_empty())
            .then(|| divergence(candidate.len(), "app.focused")))
    })
    .expect("the oracle never fails");
    assert!(shrunk.confirmed < CONFIRMATIONS && !shrunk.is_confirmed());
}

pub(crate) fn a_perturbation_waits_for_its_step_and_its_state() {
    let perturbation = Perturbation::parse(
        r#"{"after":{"key":"Left"},"while":{"app.focused":"notes.txt","app.panes[1].name":"documents"},"field":"app.focused","value":"perturbed"}"#,
    )
    .expect("the perturbation should parse");
    let before = observed(json!({
        "focused": "notes.txt",
        "panes": [{"name": "$root"}, {"name": "documents"}],
    }));
    let elsewhere = observed(json!({"focused": "todo.txt", "panes": [{"name": "$root"}]}));
    let left = json!({"key": "Left", "when": {"focused_pane": true}});
    for (step, before, changed) in [
        (&left, &before, true),
        (&json!({"key": "Right"}), &before, false),
        (&left, &elsewhere, false),
    ] {
        let mut after = observed(json!({"focused": "documents"}));
        assert_eq!(
            perturbation.apply(step, before, &mut after),
            changed,
            "{step}"
        );
        let focused = if changed { "perturbed" } else { "documents" };
        assert_eq!(after["app"]["focused"], focused);
    }
    assert!(Perturbation::parse(r#"{"after":{"key":"Left"}}"#).is_err());
}

pub(crate) fn the_map_and_the_mask_take_one_sorted_entry_to_a_line() {
    let text = "# why\napp.field.caret timing lgse/strata#1\n\n\
                app.panes[].focused a11y lgse/strata#22\nfs oracle lgse/strata#3\n";
    let entries = rules::parse(text, &EXCEPTIONS).expect("the entries should parse");
    assert_eq!(
        entries[1],
        Entry {
            field: "app.panes[].focused".to_owned(),
            code: "a11y".to_owned(),
            issue: "lgse/strata#22".to_owned(),
        }
    );
    assert_eq!(
        rules::counts(&entries, &EXCEPTIONS),
        "3 (timing 1, a11y 1, oracle 1)"
    );
    assert_eq!(rules::counts(&[], &EXCEPTIONS), "0");
    let ignored = Ignored {
        rules: entries,
        masks: Vec::new(),
    };
    assert!(ignored.covers("app.panes[3].focused") && !ignored.covers("app.focused"));
    let one = "app.focused timing lgse/strata#1\n";
    for (text, codes, expected) in [
        (
            format!("fs oracle lgse/strata#3\n{one}"),
            &EXCEPTIONS[..],
            "sorted by field",
        ),
        (format!("{one}{one}"), &EXCEPTIONS[..], "listed once"),
        (
            "app.focused timing\n".to_owned(),
            &EXCEPTIONS[..],
            "<field> <code> <issue>",
        ),
        (
            one.replace("timing", "noise"),
            &EXCEPTIONS[..],
            "the code is one of",
        ),
        (
            one.replace("lgse/strata", ""),
            &EXCEPTIONS[..],
            "lgse/strata#<number>",
        ),
        (
            one.replace("focused", "rows"),
            &EXCEPTIONS[..],
            "names no field",
        ),
        (
            one.replace("focused", "panes[0].focused"),
            &EXCEPTIONS[..],
            "written with []",
        ),
        // A mask is not an exception, and an exception is not a mask.
        (one.to_owned(), &CAUSES[..], "the code is one of"),
        (
            one.replace("timing", "unsettled"),
            &EXCEPTIONS[..],
            "the code is one of",
        ),
    ] {
        let error = rules::parse(&text, codes).expect_err(expected);
        assert!(
            error.contains(expected),
            "{error:?} should say {expected:?}"
        );
    }
    let parked = "# kept\n000000000000002a oracle lgse/strata#9\n\
                  00000000000000ff divergence lgse/strata#10\n";
    assert_eq!(
        parse_parked(parked).map(|parked| parked.iter().map(|one| one.seed).collect::<Vec<_>>()),
        Ok(vec![42, 255])
    );
    for text in [
        "2a oracle lgse/strata#9\n",
        "000000000000002a noise lgse/strata#9\n",
        "00000000000000ff oracle lgse/strata#1\n000000000000002a oracle lgse/strata#2\n",
    ] {
        assert!(parse_parked(text).is_err(), "{text}");
    }
}
