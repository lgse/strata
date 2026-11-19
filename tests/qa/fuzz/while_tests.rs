// SPDX-License-Identifier: MIT

//! A map rule's `while`, held to each side's observation: lgse/strata#2595's focus rules.

use serde_json::{Value, json};

use super::{
    compare::compare,
    compare_tests::observed,
    observe_tests::stream,
    replay::Stream,
    rules::{self, Ignored, exceptions},
};

/// Two streams, GTK's and Rune's, whose one observation shows the `app` given, and the
/// windows Rune has open beside the browser (`context.toplevels`).
fn app_streams(
    (gtk, rune, rune_toplevels, gtk_window_focused): (Value, Value, Value, bool),
) -> (Stream, Stream) {
    let (mut left, mut right) = (stream(&["archive"]), stream(&["archive"]));
    right.header.target = "rune".to_owned();
    left.observations[0].value = observed(gtk);
    left.observations[0].value["context"] =
        json!({ "toplevels": [], "window_focused": gtk_window_focused });
    right.observations[0].value = observed(rune);
    right.observations[0].value["context"] =
        json!({ "toplevels": rune_toplevels, "window_focused": true });
    (left, right)
}

/// lgse/strata#2595's two rulings on `app.focused`, as the map has them.
const FOCUS_RULES: [&str; 2] = [
    r#"app.focused toolkit lgse/strata#2595 {"gtk":null,"while":{"gtk":{"app.focused_pane":null,"context.window_focused":false}}}"#,
    r#"app.focused toolkit lgse/strata#2595 {"rune":[null],"while":{"rune":{"app.focused_pane":null,"context.toplevels":{"one_of":[["Copy to"],["Move to"]]}}}}"#,
];

/// GTK's and Rune's focus at one step, the windows Rune has open beside its browser, whether
/// GTK's window has the keyboard, and the field that still differs under lgse/strata#2595's
/// rules, if any.
type FocusCase = (Value, Value, Value, bool, Option<&'static str>);

fn focus_cases() -> Vec<FocusCase> {
    let mut cases = gtk_focus_cases();
    cases.extend(rune_focus_cases());
    cases
}

/// The cases of GTK's side: its focus excused only while its window has no keyboard.
fn gtk_focus_cases() -> Vec<FocusCase> {
    let focus_at = |pane: Value, focused: Value, directory: Value| json!({ "focused_pane": pane, "focused": focused, "directory": directory });
    let none = json!([]);
    vec![
        // GTK's window has no keyboard: its focus and directory against Rune's are excused,
        // but app.focused_pane itself is no rule's here.
        (
            focus_at(Value::Null, Value::Null, json!("documents")),
            focus_at(json!("$root"), json!("readme.md"), json!("$root")),
            none.clone(),
            false,
            Some("app.focused_pane"),
        ),
        (
            focus_at(Value::Null, Value::Null, json!("documents")),
            focus_at(Value::Null, json!("readme.md"), json!("$root")),
            none.clone(),
            false,
            None,
        ),
        // GTK's own row against Rune's is no one's.
        (
            focus_at(Value::Null, json!("todo.txt"), json!("documents")),
            focus_at(Value::Null, json!("readme.md"), json!("documents")),
            none.clone(),
            false,
            Some("app.focused"),
        ),
        // GTK has a focused pane: nothing is excused.
        (
            focus_at(json!("$root"), Value::Null, json!("$root")),
            focus_at(json!("$root"), json!("readme.md"), json!("$root")),
            none.clone(),
            true,
            Some("app.focused"),
        ),
        (
            focus_at(json!("$root"), Value::Null, json!("documents")),
            focus_at(json!("$root"), Value::Null, json!("$root")),
            none.clone(),
            true,
            Some("app.directory"),
        ),
        // GTK's keyboard on the sidebar: its window has the keyboard, so no rule applies, and
        // Rune's keyboard left on a row still counts.
        (
            focus_at(Value::Null, Value::Null, json!("$root")),
            focus_at(json!("$root"), json!("readme.md"), json!("$root")),
            none,
            true,
            Some("app.focused"),
        ),
    ]
}

/// The cases of Rune's side: its focus excused only while its browser has none behind an open
/// floating chooser.
fn rune_focus_cases() -> Vec<FocusCase> {
    let focus_at = |pane: Value, focused: Value, directory: Value| json!({ "focused_pane": pane, "focused": focused, "directory": directory });
    let gtk = focus_at(json!("$root"), json!("readme.md"), json!("$root"));
    let (chooser, none) = (json!(["Copy to"]), json!([]));
    vec![
        // Rune's browser has no focus behind its open chooser: GTK's row is excused.
        (
            gtk.clone(),
            focus_at(Value::Null, Value::Null, json!("$root")),
            chooser.clone(),
            true,
            Some("app.focused_pane"),
        ),
        // The same with no chooser open on Rune: it counts.
        (
            gtk.clone(),
            focus_at(Value::Null, Value::Null, json!("$root")),
            none,
            true,
            Some("app.focused"),
        ),
        // Rune's browser kept the keyboard behind its chooser: it counts.
        (
            gtk,
            focus_at(json!("$root"), json!("todo.txt"), json!("$root")),
            chooser,
            true,
            Some("app.focused"),
        ),
    ]
}

/// A rule may leave out either side's values, and a `while` applies it only at a step at which
/// that side's observation holds it (lgse/strata#2595). GTK's no focus is excused while GTK's
/// browser has no focused pane; GTK's row against Rune's no focus only while Rune's has none
/// and a floating chooser is open. A Rune browser that kept the keyboard behind the chooser,
/// or lost it with no chooser open, still differs, as does anything where the comparison does
/// not know which stream is whose.
pub(crate) fn a_rule_s_while_holds_it_to_each_side_s_observation() {
    let lines = [
        r#"app.directory toolkit lgse/strata#2595 {"while":{"gtk":{"app.focused_pane":null,"context.window_focused":false}}}"#,
        FOCUS_RULES[0],
        FOCUS_RULES[1],
    ];
    let ruled = Ignored {
        rules: rules::parse(&lines.join("\n"), &exceptions()).expect("the rules should parse"),
        masks: Vec::new(),
    };
    for (gtk, rune, toplevels, window, differs) in focus_cases() {
        let (left, right) = app_streams((gtk.clone(), rune.clone(), toplevels.clone(), window));
        let found = compare(&left, &right, &ruled).map(|found| found.field().to_owned());
        assert_eq!(
            found.as_deref(),
            differs,
            "gtk {gtk} against rune {rune}, {toplevels}"
        );
        // The order of the streams does not turn the rules round.
        let found = compare(&right, &left, &ruled).map(|found| found.field().to_owned());
        assert_eq!(
            found.as_deref(),
            differs,
            "rune {rune} against gtk {gtk}, {toplevels}"
        );
    }
    // Two streams of one target name no GTK or Rune stream, so no `while` applies.
    let (mut left, right) = app_streams((
        json!({ "focused_pane": null, "focused": null, "directory": "documents" }),
        json!({ "focused_pane": "$root", "focused": "readme.md", "directory": "$root" }),
        json!([]),
        false,
    ));
    left.header.target = "rune".to_owned();
    assert!(compare(&left, &right, &ruled).is_some());
    // Two rules of one field, code and issue are ordered by what follows the issue.
    let reversed = format!("{}\n{}", FOCUS_RULES[1], FOCUS_RULES[0]);
    let error = rules::parse(&reversed, &exceptions()).expect_err("out of order");
    assert!(error.contains("then by what follows the issue"), "{error}");
}
