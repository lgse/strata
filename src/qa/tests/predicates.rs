// SPDX-License-Identifier: MIT

use serde_json::json;

use super::{
    super::{
        note::Shape,
        protocol::{Change, Check, Names, Predicate, Target, TextMatch},
    },
    predicate,
};

#[test]
fn a_note_shapes_a_list_as_a_slice_step_reads_it() {
    let names: Vec<String> = ["010", "011", "012", "013", "014", "015", "016"]
        .map(str::to_owned)
        .to_vec();
    fn shape(middle: Option<usize>, plus: &[String], sorted: bool, pick: Option<i64>) -> Shape<'_> {
        Shape {
            from: None,
            through: None,
            middle,
            plus,
            sorted,
            pick,
        }
    }
    assert_eq!(
        shape(Some(3), &[], false, None).apply(names.clone()),
        Ok(json!(["013", "014", "015"]))
    );
    assert_eq!(
        shape(Some(3), &[], false, Some(-1)).apply(names.clone()),
        Ok(json!("015"))
    );
    assert_eq!(
        shape(
            None,
            &["todo.txt".to_owned(), "011".to_owned()],
            false,
            None
        )
        .apply(vec!["zz".to_owned(), "011".to_owned()]),
        Ok(json!(["011", "todo.txt", "zz"]))
    );
    assert_eq!(
        shape(None, &[], true, Some(0)).apply(vec!["b".to_owned(), "a".to_owned()]),
        Ok(json!("a"))
    );
    assert!(shape(None, &[], false, Some(9)).apply(names).is_err());
}

#[test]
fn name_filters_compare_names_as_python_strings_do() {
    let actual: Vec<String> = ["011.txt", "040.txt", "041.txt"]
        .map(str::to_owned)
        .to_vec();
    let names = |json: serde_json::Value| -> Names {
        serde_json::from_value(json).expect("the names should parse")
    };
    for holding in [
        json!({ "first": "011.txt" }),
        json!({ "first_above": "010.txt" }),
        json!({ "any_from": "040.txt" }),
        json!({ "toggled": { "names": ["011.txt", "000.txt"], "against": ["000.txt"] } }),
        json!({ "in_order": ["011.txt", "041.txt"] }),
        json!({ "set": ["041.txt", "011.txt", "040.txt"] }),
        json!({ "occurrences": { "011.txt": 1, "000.txt": 0 } }),
    ] {
        assert!(
            names(holding.clone()).matches(&actual),
            "{holding} should hold"
        );
    }
    for failing in [
        json!({ "first": "040.txt" }),
        json!({ "first_above": "011.txt" }),
        json!({ "any_from": "042.txt" }),
        json!({ "toggled": { "names": ["011.txt"], "against": ["011.txt"] } }),
        json!({ "in_order": ["041.txt", "011.txt"] }),
        json!({ "in_order": ["011.txt", "042.txt"] }),
        json!({ "set": ["011.txt", "040.txt"] }),
        json!({ "set": ["011.txt", "040.txt", "041.txt", "042.txt"] }),
        json!({ "occurrences": { "011.txt": 2 } }),
    ] {
        assert!(
            !names(failing.clone()).matches(&actual),
            "{failing} should not hold"
        );
    }
    assert!(Change::Exactly(0).matches(0));
    assert!(!Change::Exactly(0).matches(1));
    let grown = Change::Bounded {
        min: Some(30),
        max: None,
    };
    assert!(grown.matches(30) && !grown.matches(29));
}

#[test]
fn screen_and_footer_predicates_parse_and_are_read_live() {
    for json in [
        json!({ "pixel": { "target": { "result": "beta/thumb.png" }, "rgb": [230, 40, 60] } }),
        json!({ "accent_pixels": { "target": { "field": "filter" }, "inset": 2, "more_than": 100 } }),
        json!({ "hit_path": { "mark": "search: txt", "hits": { "todo.txt": "todo.txt" }, "name": "todo.txt" } }),
        json!({ "focused_widget": { "contains": "Shortcuts" } }),
        json!({ "mentions": { "target": { "dialog": true }, "role": "label", "text": "permission" } }),
    ] {
        assert!(
            predicate(json.clone()).check.is_live(),
            "{json} reads the live window"
        );
    }
    let Check::FocusedWidget(Some(TextMatch::Like(pattern))) =
        predicate(json!({ "focused_widget": { "contains": "Shortcuts" } })).check
    else {
        panic!("a focused_widget pattern should parse as a text pattern");
    };
    assert_eq!(pattern.contains.as_deref(), Some("Shortcuts"));
}

#[test]
fn numbers_compare_and_parse_from_their_operations() {
    use super::super::protocol::{CompareOp, Expr, Measure};
    assert!(CompareOp::AtMost.holds(2.0, 2.0));
    assert!(!CompareOp::Less.holds(2.0, 2.0));
    assert!(CompareOp::Unequal.holds(560.0, 412.0));
    let compare = predicate(json!({"compare": {
        "left": {"abs": {"minus": [{"of": {"preview": true}, "get": "x"}, {"of": {"pane": null}, "get": "right"}]}},
        "op": "<=",
        "right": 3,
    }}));
    let Check::Compare(comparison) = &compare.check else {
        panic!("compare parses as a comparison");
    };
    assert_eq!(comparison.op, CompareOp::AtMost);
    let Expr::Abs(inner) = &comparison.left else {
        panic!("abs wraps the difference");
    };
    let Expr::Minus(pair) = &**inner else {
        panic!("minus takes two numbers");
    };
    assert_eq!(
        pair[0],
        Expr::Of {
            target: Box::new(Target::Preview),
            get: Measure::X
        }
    );
    assert_eq!(
        serde_json::to_value(&compare).expect("a predicate serializes")["compare"]["left"],
        json!({"abs": {"minus": [{"of": {"preview": true}, "get": "x"}, {"of": {"pane": null}, "get": "right"}]}}),
        "a number serializes back to its spelling"
    );
    for bad in [
        json!({"compare": {"left": {"of": {"preview": true}}, "op": "<", "right": 1}}),
        json!({"compare": {"left": {"minus": [1]}, "op": "<", "right": 1}}),
        json!({"compare": {"left": {"pow": [1, 2]}, "op": "<", "right": 1}}),
        json!({"compare": {"left": 1, "op": "=<", "right": 1}}),
    ] {
        assert!(
            serde_json::from_value::<Predicate>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    assert!(serde_json::from_value::<Predicate>(json!({"compare": {"left": {"count": {"description": "Columns view", "rendered": false}}, "op": "==", "right": {"pref": "text_size", "default": 0}}})).is_ok());
}

#[test]
fn name_filters_take_a_leading_run() {
    let names: Vec<String> = ["Home", "Trash", "Network"].map(str::to_owned).into();
    let leading = |first: &[&str]| {
        Names::Including(Box::new(
            serde_json::from_value(json!({ "starts_with": first })).expect("a name filter"),
        ))
    };
    assert!(leading(&["Home", "Trash"]).matches(&names));
    assert!(!leading(&["Trash", "Home"]).matches(&names));
}
