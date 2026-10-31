// SPDX-License-Identifier: MIT

use serde_json::json;

use super::super::{
    pointer::step_towards,
    protocol::{At, Axis, Command, EntryPart, EntryTarget, Offset, Target, parse_command},
    targets::Rect,
};

#[test]
fn every_target_kind_parses_from_its_key() {
    let target = |json: &str| -> Target {
        let Ok(Command::Move { target, .. }) = parse_command(&format!(r#"{{"move":{json}}}"#))
        else {
            panic!("{json} should be a target");
        };
        target
    };
    assert_eq!(
        target(r#"{"entry":"todo.txt","dir":"archive"}"#),
        Target::Entry(EntryTarget {
            entry: "todo.txt".to_owned(),
            dir: Some("archive".to_owned()),
            part: None,
        })
    );
    assert_eq!(
        target(r#"{"role":"button","name":"Keep Both"}"#),
        Target::Role {
            role: "button".to_owned(),
            other_roles: Vec::new(),
            name: Some("Keep Both".to_owned()),
            description: None,
            within: None,
            name_ends_with: Vec::new(),
            nth: None,
            states: Vec::new(),
            name_matches: None,
            has: None,
            rendered: true,
        }
    );
    assert_eq!(
        target(r#"{"background":true,"dir":"archive"}"#),
        Target::Background {
            dir: Some("archive".to_owned())
        }
    );
    assert_eq!(target(r#"{"pane":null}"#), Target::Pane { dir: None });
    assert_eq!(
        target(r#"{"viewport":"archive"}"#),
        Target::Viewport {
            dir: Some("archive".to_owned())
        }
    );
    assert_eq!(target(r#"{"field":true}"#), Target::Field(None));
    assert_eq!(
        target(r#"{"field":"filter"}"#),
        Target::Field(Some("filter".to_owned()))
    );
    assert_eq!(
        target(r#"{"result":"beta/match-note.txt"}"#),
        Target::Result("beta/match-note.txt".to_owned())
    );
    assert_eq!(
        target(r#"{"result_row":-1,"containing":"match-note"}"#),
        Target::ResultRow {
            index: -1,
            containing: Some("match-note".to_owned()),
            name: None,
        }
    );
    assert_eq!(
        target(r#"{"role":"list item","name_ends_with":["/a"],"nth":-1}"#),
        Target::Role {
            role: "list item".to_owned(),
            other_roles: Vec::new(),
            name: None,
            description: None,
            within: None,
            name_ends_with: vec!["/a".to_owned()],
            nth: Some(-1),
            states: Vec::new(),
            name_matches: None,
            has: None,
            rendered: true,
        }
    );
    assert_eq!(target(r#"{"focused_widget":true}"#), Target::FocusedWidget);
    assert_eq!(
        target(r#"{"remembered":"focused"}"#),
        Target::Remembered("focused".to_owned())
    );
    assert_eq!(
        target(r#"{"sidebar":"Home"}"#),
        Target::Sidebar("Home".to_owned())
    );
    assert_eq!(
        target(r#"{"header":"Size"}"#),
        Target::Header("Size".to_owned())
    );
    assert_eq!(
        target(r#"{"menu_item":"New File"}"#),
        Target::MenuItem("New File".to_owned())
    );
    assert_eq!(
        target(r#"{"dialog_button":"Close"}"#),
        Target::DialogButton("Close".to_owned())
    );
    for json in [
        r#"{"entry":"todo.txt","dir":"archive"}"#,
        r#"{"background":true}"#,
        r#"{"pane":null}"#,
        r#"{"menu_item":"New File"}"#,
        r#"{"role":"button","name":"Next video","within":{"preview":true}}"#,
        r#"{"field":"filter"}"#,
        r#"{"result":"beta/match-note.txt"}"#,
        r#"{"result_row":0,"name":"match-note.txt"}"#,
        r#"{"role":"button","name":"drafts","nth":-1}"#,
        r#"{"role":"list item","name_ends_with":["/a","/b"]}"#,
        r#"{"focused_widget":true}"#,
        r#"{"remembered":"focused"}"#,
    ] {
        assert_eq!(
            target(json).to_json(),
            serde_json::from_str::<serde_json::Value>(json).expect("the JSON should parse"),
            "{json} should round-trip, since settle keys targets by their JSON"
        );
    }
}

#[test]
fn a_bad_target_names_the_kind_and_the_problem() {
    let error = |line: &str| parse_command(line).expect_err("the target should be rejected");
    assert!(
        error(r#"{"click":{"entry":"a","name":"b"}}"#).contains("invalid entry target"),
        "an unknown field names the target kind"
    );
    assert!(error(r#"{"click":{"name":"b"}}"#).contains("a target needs exactly one of"));
    assert!(error(r#"{"click":"todo.txt"}"#).contains("a target is an object"));
    assert!(
        error(r#"{"click":{"result_row":0,"containing":"a","name":"b"}}"#)
            .contains("containing or name, not both")
    );
    assert!(error(r#"{"click":{"focused_widget":false}}"#).contains("must be true"));
}

#[test]
fn at_offsets_resolve_like_the_python_points() {
    let bounds = Rect {
        x: 100.0,
        y: 40.0,
        width: 301.0,
        height: 95.0,
    };
    assert_eq!(bounds.point(None), (250.0, 87.0));
    let at = |x: Offset, y: Offset| At { x, y };
    assert_eq!(
        bounds.point(Some(&at(Offset::Percent(50), Offset::Percent(80)))),
        (250.0, 116.0)
    );
    assert_eq!(
        bounds.point(Some(&at(Offset::Pixels(4), Offset::Pixels(-14)))),
        (104.0, 121.0)
    );
    assert_eq!(
        bounds.point(Some(&at(Offset::Fraction(2, 3), Offset::FromStart(-6)))),
        (300.0, 34.0)
    );
    let Ok(Command::Move {
        at: Some(parsed), ..
    }) = parse_command(r#"{"move":{"pane":null},"at":["2/3","start-6"]}"#)
    else {
        panic!("fractions and near-edge offsets should parse");
    };
    assert_eq!(parsed, at(Offset::Fraction(2, 3), Offset::FromStart(-6)));
    assert_eq!(
        bounds.point(Some(&at(
            Offset::FractionPlus(2, 3, 3),
            Offset::EndLessFraction(1, 6)
        ))),
        (303.0, 120.0)
    );
    assert_eq!(
        bounds.point(Some(&at(Offset::FromEnd(60), Offset::FromEnd(-8)))),
        (461.0, 127.0)
    );
    let Ok(Command::Move {
        at: Some(parsed), ..
    }) = parse_command(r#"{"move":{"pane":null},"at":["2/3+3","end-1/6"]}"#)
    else {
        panic!("shares with pixels and shares from the far edge should parse");
    };
    assert_eq!(
        parsed,
        at(Offset::FractionPlus(2, 3, 3), Offset::EndLessFraction(1, 6))
    );
    let Ok(Command::Move {
        at: Some(parsed), ..
    }) = parse_command(r#"{"move":{"pane":null},"at":["end+60","end-2"]}"#)
    else {
        panic!("far-edge offsets should parse");
    };
    assert_eq!(parsed, at(Offset::FromEnd(60), Offset::FromEnd(-2)));
    for bad in [
        r#"["3/2",0]"#,
        r#"["end2",0]"#,
        r#"["start6",0]"#,
        r#"["2/3+x",0]"#,
        r#"["end+1/6",0]"#,
    ] {
        assert!(
            parse_command(&format!(r#"{{"move":{{"pane":null}},"at":{bad}}}"#)).is_err(),
            "{bad} should be rejected"
        );
    }
}

#[test]
fn drag_steps_floor_towards_the_target_as_python_does() {
    let path: Vec<i16> = (1..=4).map(|step| step_towards(100, 10, step, 4)).collect();
    assert_eq!(path, [77, 55, 32, 10]);
    let path: Vec<i16> = (1..=3).map(|step| step_towards(10, 20, step, 3)).collect();
    assert_eq!(path, [13, 16, 20]);
}

#[test]
fn composed_and_kept_targets_parse_and_round_trip() {
    let target = |json: &str| -> Target {
        let Ok(Command::Move { target, .. }) = parse_command(&format!(r#"{{"move":{json}}}"#))
        else {
            panic!("{json} should be a target");
        };
        target
    };
    assert_eq!(
        target(
            r#"{"point":[{"of":{"pane":null},"at":2},{"mid":[{"of":{"kept":"a"},"at":"end+0"},{"of":{"kept":"b"}}],"plus":40}]}"#
        ),
        Target::Point(Box::new([
            Axis::Of {
                target: Box::new(Target::Pane { dir: None }),
                at: Some(Offset::Pixels(2)),
                plus: 0,
            },
            Axis::Mid {
                pair: Box::new([
                    Axis::Of {
                        target: Box::new(Target::Kept("a".to_owned())),
                        at: Some(Offset::FromEnd(0)),
                        plus: 0,
                    },
                    Axis::Of {
                        target: Box::new(Target::Kept("b".to_owned())),
                        at: None,
                        plus: 0,
                    },
                ]),
                plus: 40,
            },
        ]))
    );
    assert_eq!(
        target(r#"{"entry":"todo.txt","part":"metadata"}"#),
        Target::Entry(EntryTarget {
            entry: "todo.txt".to_owned(),
            dir: None,
            part: Some(EntryPart::Metadata),
        })
    );
    for json in [
        r#"{"search_result":"todo.txt"}"#,
        r#"{"named":"Apply to All","within":{"dialog":true}}"#,
        r#"{"container":"archive"}"#,
        r#"{"dialog":true}"#,
        r#"{"parent":{"sidebar":"Home"}}"#,
        r#"{"ancestor":"scroll pane","of":{"pane":"deep"}}"#,
        r#"{"kept":"edge"}"#,
        r#"{"point":[{"extent":{"window":true},"plus":-4},{"extent":{"window":true},"plus":-4}]}"#,
        r#"{"point":[{"max":[{"of":{"pane":"a"},"at":10},{"of":{"viewport":null},"at":"4/5"}]},{"of":{"pane":"a"},"at":"50%"}]}"#,
        r#"{"role":"panel","within":{"entry":"a","dir":"b"}}"#,
        r#"{"role":"label","name":"b","description":"/a/b","within":{"toplevel":"Copy to"}}"#,
        r#"{"field":true,"within":{"toplevel":"Copy to"}}"#,
    ] {
        assert_eq!(
            target(json).to_json(),
            serde_json::from_str::<serde_json::Value>(json).expect("the JSON should parse"),
            "{json} should round-trip, since settle keys targets by their JSON"
        );
    }
    for bad in [
        r#"{"point":[{"of":{"pane":null}}]}"#,
        r#"{"point":[{"mid":[{"of":{"pane":null}}]},{"of":{"pane":null}}]}"#,
        r#"{"point":[{"mid":[{"of":{"pane":null}},{"of":{"pane":null}}],"at":2},{"of":{"pane":null}}]}"#,
        r#"{"point":[{"extent":{"window":true},"at":2},{"of":{"pane":null}}]}"#,
        r#"{"parent":{"sidebar":"Home"},"dir":"a"}"#,
        r#"{"dialog":false}"#,
        r#"{"field":"a","within":{"dialog":true}}"#,
        r#"{"toplevel":true}"#,
    ] {
        assert!(
            parse_command(&format!(r#"{{"move":{bad}}}"#)).is_err(),
            "{bad} should be rejected"
        );
    }
}

#[test]
fn role_targets_carry_states_and_name_patterns() {
    let target: Target = serde_json::from_value(
        json!({"role": "page tab", "name": "docs", "states": ["selected"], "name_matches": "^d"}),
    )
    .expect("the json should parse");
    assert_eq!(
        target.to_json(),
        json!({"role": "page tab", "name": "docs", "states": ["selected"], "name_matches": "^d"})
    );
}
