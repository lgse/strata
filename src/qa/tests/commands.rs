// SPDX-License-Identifier: MIT

use std::time::Duration;

use serde_json::json;

use super::{
    super::{
        input,
        protocol::{
            Accessor, At, Check, Chord, Command, EntryTarget, Modifier, Offset, Predicate,
            Presence, Query, StateName, Target, TextMatch, changed_keys, parse_command,
        },
    },
    predicate,
};

#[test]
fn commands_parse_from_the_scenario_spelling_with_their_defaults() {
    assert_eq!(
        parse_command(r#"{"key":"ctrl+shift+n"}"#),
        Ok(Command::Key {
            chord: Chord {
                modifiers: vec![Modifier::Ctrl, Modifier::Shift],
                key: "n".to_owned(),
            },
            repeat: 1,
        })
    );
    assert_eq!(
        parse_command(r#"{"wait":{"focused":"todo.txt"}}"#),
        Ok(Command::Wait {
            predicate: Predicate {
                check: Check::Focused(Presence::Named("todo.txt".to_owned())),
                dir: None,
            },
            timeout: Duration::from_millis(15_000),
            message: None,
            samples: 1,
            settle: None,
            never: None,
        })
    );
    assert_eq!(
        parse_command(r#"{"click":{"entry":"readme.md","dir":"docs"}}"#),
        Ok(Command::Click {
            target: Target::Entry(EntryTarget {
                entry: "readme.md".to_owned(),
                dir: Some("docs".to_owned()),
                part: None,
            }),
            button: 1,
            count: 1,
            modifiers: Vec::new(),
            at: None,
            left_bound: None,
            release_modifiers_first: false,
        })
    );
    assert_eq!(
        parse_command(r#"{"query":"tree"}"#),
        Ok(Command::Query(Query::Tree))
    );
    assert_eq!(
        parse_command(r#"{"select":{"entry":"readme.md"}}"#),
        Ok(Command::Select(EntryTarget {
            entry: "readme.md".to_owned(),
            dir: None,
            part: None,
        }))
    );
    assert_eq!(
        parse_command(r#"{"wheel":{"viewport":null},"at":["50%",-14],"direction":"up"}"#),
        Ok(Command::Wheel {
            target: Target::Viewport { dir: None },
            at: Some(At {
                x: Offset::Percent(50),
                y: Offset::Pixels(-14),
            }),
            clicks: 3,
            down: false,
            modifiers: Vec::new(),
            until: None,
        })
    );
    assert_eq!(
        parse_command(r#"{"activate":{"role":"toggle button","name":"Single"},"message":"m"}"#),
        Ok(Command::Activate {
            target: Target::Role {
                role: "toggle button".to_owned(),
                other_roles: Vec::new(),
                name: Some("Single".to_owned()),
                description: None,
                within: None,
                name_ends_with: Vec::new(),
                nth: None,
                states: Vec::new(),
                name_matches: None,
                has: None,
                rendered: true,
            },
            message: Some("m".to_owned()),
        })
    );
    assert_eq!(
        parse_command(r#"{"holds":{"dialog":null}}"#),
        Ok(Command::Holds(Predicate {
            check: Check::Dialog(Presence::Absent),
            dir: None,
        }))
    );
    let Ok(Command::Expect {
        predicates,
        message: None,
        timeout: None,
        hold: None,
    }) = parse_command(r#"{"expect":[{"selected":["a"],"dir":"x"},{"focused":null}]}"#)
    else {
        panic!("expect should take a list of predicates");
    };
    assert_eq!(predicates.len(), 2);
    let Ok(Command::Expect { message, .. }) =
        parse_command(r#"{"expect":{"focused":null},"message":"focus to leave the list"}"#)
    else {
        panic!("expect should take an assertion message");
    };
    assert_eq!(message.as_deref(), Some("focus to leave the list"));
}

#[test]
fn malformed_commands_are_rejected_instead_of_guessed() {
    for line in [
        r#"{"key":"Down","wait":{"focused":"a"}}"#,
        r#"{"key":"Down","repeats":2}"#,
        r#"{"wait":{"focused":"a","directory":"b"}}"#,
        r#"{"wait":{"directory":"b","dir":"c"}}"#,
        r#"{"click":{"entry":"a","name":"b"}}"#,
        r#"{"click":{"entry":"a"},"button":4}"#,
        r#"{"click":{"entry":"a"},"button":2}"#,
        r#"{"click":{"background":true},"at":[1,2]}"#,
        r#"{"click":{"background":false}}"#,
        r#"{"click":{"sidebar":"Home","header":"Size"}}"#,
        r#"{"click":{"entry":"a"},"at":[1]}"#,
        r#"{"click":{"entry":"a"},"at":["101%",0]}"#,
        r#"{"move":{"pane":"a","dir":"b"}}"#,
        r#"{"wheel":{"viewport":null},"clicks":0}"#,
        r#"{"wheel":{"viewport":null},"direction":"sideways"}"#,
        r#"{"drag":{"from":{"entry":"a"}}}"#,
        r#"{"drag":{"from":{"entry":"a"},"to":{"entry":"b"}},"steps":0}"#,
        r#"{"resize":{"width":0,"height":300}}"#,
        r#"{"wait":{"present":{"entry":"a"}},"dir":"x"}"#,
        r#"{"wait":{"moved":{"target":{"entry":"a"},"direction":"diagonal"}}}"#,
        r#"{"key":"hyper+a"}"#,
        r#"{"expect":[]}"#,
        r#"{"type":""}"#,
        r#"{"activate":{"entry":"a"}}"#,
        r#"{"remember":{"field":"kept"},"as":"x"}"#,
        r#"{"note":"entries"}"#,
        r#"{"note":"directory","dir":"x","as":"a"}"#,
        r#"{"note":"focused","from":"a","as":"b"}"#,
        r#"{"note":"middle_visible","as":"a"}"#,
        r#"{"note":"entries","unless_first":"a","as":"b"}"#,
        r#"{"note":"entries","message":"m","as":"a"}"#,
        r#"{"click":{"entry":"a"},"release_mods_first":true}"#,
        r#"{"click":{"entry":"a"},"count":2,"mods":["shift"],"release_mods_first":true}"#,
        r#"{"click":{"window":false}}"#,
        r#"{"click":{"entry":"a","part":"name"}}"#,
        r#"{"button_down":4}"#,
        r#"{"set_scrollbar":"diagonal","value":0}"#,
        r#"{"wait":{"focused":"a"},"samples":0}"#,
        r#"{"wait":{"focused":{"one_of":"a"}}}"#,
        r#"{"wait":{"not":{"selected":[]},"dir":"x"}}"#,
        r#"{"drag":{"from":{"entry":"a"},"to":{"entry":"b"}},"message":"m"}"#,
        r#"{"remember":{"field":true}}"#,
        r#"{"click":{"field":false}}"#,
        r#"{"wait":{"dialog":false}}"#,
        r#"{"wait":{"field":{"text":"a","caret":"end"}}}"#,
        r#"{"wait":{"pref":{"key":"browser_mode"}}}"#,
        r#"{"open_menu":{"header":"Appearance"}}"#,
        r#"{"wait":{"field":{"of":"rename","name":"Rename"}}}"#,
        r#"{"expect":{"editor_within_viewport":{"of":"rename","name":"Rename"}}}"#,
        r#"{"wait":{"any":[{"field":{"of":"a","name":"b"}}]}}"#,
        r#"{"holds":{"focused":"a"},"message":"b"}"#,
        "[]",
    ] {
        assert!(parse_command(line).is_err(), "{line} should be rejected");
    }
}

#[test]
fn a_note_waits_only_when_asked_and_a_drag_checks_after_its_press() {
    let Ok(Command::Note {
        value,
        dir,
        wait,
        from,
        through,
        ..
    }) = parse_command(
        r#"{"note":"entries","dir":"root","from":"documents","through":"todo.txt","as":"range"}"#,
    )
    else {
        panic!("a sliced note should parse");
    };
    assert_eq!(value, Accessor::Entries);
    assert_eq!(dir.as_deref(), Some("root"));
    assert_eq!(wait, None);
    assert_eq!(
        (from.as_deref(), through.as_deref()),
        (Some("documents"), Some("todo.txt"))
    );
    let Ok(Command::Note { wait, .. }) = parse_command(
        r#"{"note":"middle_visible","unless_first":"folder-000","until_found":true,"as":"first"}"#,
    ) else {
        panic!("a waiting note should parse");
    };
    assert_eq!(wait, Some(Duration::from_millis(15_000)));
    let Ok(Command::Drag {
        from_at,
        after_press,
        ..
    }) = parse_command(
        r#"{"drag":{"from":{"pane":"a"},"from_at":["50%",-60],"to":{"entry":"b"}},"after_press":{"selected":[],"dir":"a"},"message":"m"}"#,
    )
    else {
        panic!("a drag with a check after its press should parse");
    };
    assert_eq!(
        from_at,
        Some(At {
            x: Offset::Percent(50),
            y: Offset::Pixels(-60)
        })
    );
    let [check] = after_press.as_slice() else {
        panic!("one predicate is one check");
    };
    assert_eq!(check.predicate.dir.as_deref(), Some("a"));
    assert_eq!((check.wait, check.message.as_deref()), (None, Some("m")));
    let Ok(Command::Drag { after_press, .. }) = parse_command(
        r#"{"drag":{"from":{"entry":"a"},"to":{"entry":"b"}},"after_press":[{"wait":{"entry":"a"},"message":"w"},{"expect":{"preview":null},"message":"e"}]}"#,
    ) else {
        panic!("a drag with a list of checks after its press should parse");
    };
    let waits: Vec<(Option<Duration>, Option<&str>)> = after_press
        .iter()
        .map(|check| (check.wait, check.message.as_deref()))
        .collect();
    assert_eq!(
        waits,
        [
            (Some(Duration::from_millis(15_000)), Some("w")),
            (None, Some("e"))
        ]
    );
    for bad in [
        r#"{"drag":{"from":{"entry":"a"},"to":{"entry":"b"}},"after_press":[{"wait":{"entry":"a"}}],"message":"m"}"#,
        r#"{"drag":{"from":{"entry":"a"},"to":{"entry":"b"}},"after_press":[{"expect":{"entry":"a"},"timeout_ms":5}]}"#,
        r#"{"drag":{"from":{"entry":"a"},"to":{"entry":"b"}},"after_press":[]}"#,
    ] {
        assert!(parse_command(bad).is_err(), "{bad} should be rejected");
    }
}

#[test]
fn a_trailing_plus_is_the_plus_key() {
    assert_eq!(
        Chord::parse("ctrl++"),
        Ok(Chord {
            modifiers: vec![Modifier::Ctrl],
            key: "+".to_owned(),
        })
    );
    assert_eq!(
        Chord::parse("+"),
        Ok(Chord {
            modifiers: Vec::new(),
            key: "+".to_owned(),
        })
    );
}

#[test]
fn single_characters_are_latin1_or_unicode_keysyms() {
    assert_eq!(input::keysym("j"), Ok(0x6a));
    assert_eq!(input::keysym("é"), Ok(0xe9));
    assert_eq!(input::keysym("о"), Ok(0x0100_043e));
    assert!(input::keysym("NotAKey").is_err());
}

#[test]
fn changed_lists_only_the_keys_that_differ() {
    let before = json!({ "directory": "a", "focused": "x", "panes": [] });
    let after = json!({ "directory": "a", "focused": "y", "panes": [] });
    assert_eq!(changed_keys(Some(&before), &after), ["focused"]);
    assert_eq!(changed_keys(None, &before).len(), 3);
}

#[test]
fn new_commands_parse_and_reject_what_they_cannot_mean() {
    let Ok(Command::Wheel {
        until: Some(repeat),
        ..
    }) = parse_command(
        r#"{"wheel":{"viewport":null},"direction":"up","until":{"peek":false},"tries":40}"#,
    )
    else {
        panic!("a bounded wheel loop should parse");
    };
    assert_eq!((repeat.tries, repeat.message), (Some(40), None));
    let Ok(Command::Wheel {
        until: Some(repeat),
        ..
    }) = parse_command(r#"{"wheel":{"pane":null},"until":{"peek":false},"message":"m"}"#)
    else {
        panic!("a timed wheel loop should parse");
    };
    assert_eq!(
        (repeat.tries, repeat.timeout, repeat.message.as_deref()),
        (None, Duration::from_millis(15_000), Some("m"))
    );
    assert_eq!(
        parse_command(r#"{"during":{"peek":false},"ms":200,"every_ms":20,"message":"m"}"#),
        Ok(Command::During {
            checks: vec![(predicate(json!({ "peek": false })), Some("m".to_owned()))],
            duration: Duration::from_millis(200),
            every: Duration::from_millis(20),
        })
    );
    let Ok(Command::KeysUntil { keys, times, .. }) = parse_command(
        r#"{"keys_until":{"state":{"target":{"sidebar":"Home"},"focused":true}},"keys":["Home","Left"],"times":2}"#,
    ) else {
        panic!("a bounded key loop should parse");
    };
    assert_eq!((keys.len(), times), (2, 2));
    let Ok(Command::KeysUntil {
        times, check_first, ..
    }) = parse_command(
        r#"{"keys_until":{"state":{"target":{"menu_item":"Actions"},"focused":true}},"keys":["Down"],"times":["Cut","Copy","Actions"],"check_first":true}"#,
    )
    else {
        panic!("a key loop over a noted list should parse");
    };
    assert_eq!((times, check_first), (3, true));
    let Ok(Command::Expect { timeout, .. }) =
        parse_command(r#"{"expect":{"focused":"a"},"timeout_ms":4000,"as":"returned"}"#)
    else {
        panic!("a polling expect should parse");
    };
    assert_eq!(timeout, Some(Duration::from_millis(4000)));
    for bad in [
        r#"{"expect":{"mentions":{"target":{"dialog":true},"text":"a","words":"a b"}}}"#,
        r#"{"expect":{"file":{"path":"todo.txt","exists":true}}}"#,
        r#"{"wait":{"all":[]}}"#,
        r#"{"wait":{"any":[]}}"#,
        r#"{"activate":{"background":true}}"#,
    ] {
        assert!(parse_command(bad).is_err(), "{bad} should be rejected");
    }
    let Ok(Command::Note { value, .. }) =
        parse_command(r#"{"note":"bounds","of":{"viewport":null},"as":"viewport_bounds"}"#)
    else {
        panic!("a bounds note should parse");
    };
    assert_eq!(value, Accessor::Bounds);
    assert_eq!(
        parse_command(r#"{"key_down":"ctrl"}"#),
        Ok(Command::KeyDown("ctrl".to_owned()))
    );
    assert_eq!(
        parse_command(r#"{"resize":{"width":600}}"#),
        Ok(Command::Resize {
            width: 600,
            height: None
        })
    );
    let Ok(Command::Note {
        value,
        middle,
        pick,
        ..
    }) = parse_command(r#"{"note":"visible","middle":3,"pick":-1,"as":"last"}"#)
    else {
        panic!("a shaped visible note should parse");
    };
    assert_eq!(
        (value, middle, pick),
        (Accessor::Visible, Some(3), Some(-1))
    );
    for bad in [
        r#"{"wheel":{"pane":null},"tries":3}"#,
        r#"{"wheel":{"pane":null},"until":{"peek":false},"tries":3,"timeout_ms":5}"#,
        r#"{"wheel":{"pane":null},"until":{"peek":false},"tries":0}"#,
        r#"{"during":{"peek":false},"ms":0}"#,
        r#"{"key_down":"ctrl+c"}"#,
        r#"{"note":"focused","pick":0,"as":"x"}"#,
        r#"{"note":"scroll_target","as":"x"}"#,
        r#"{"note":"bounds","as":"x"}"#,
        r#"{"keys_until":{"peek":true},"keys":[],"times":2}"#,
        r#"{"keys_until":{"peek":true},"keys":["Home"],"times":0}"#,
        r#"{"keys_until":{"peek":true},"keys":["Home"],"times":[]}"#,
        r#"{"note":"entries","of":{"entry":"a"},"as":"x"}"#,
        r#"{"capture":{"kept":"a"},"as":""}"#,
        r#"{"wait":{"band_selected":{"from":"a"}}}"#,
    ] {
        assert!(parse_command(bad).is_err(), "{bad} should be rejected");
    }
}

#[test]
fn result_row_states_parse_and_need_a_state() {
    let Ok(Command::ClickResultRows { states, modifiers }) =
        parse_command(r#"{"click_result_rows":{"selected":true},"mods":["ctrl"]}"#)
    else {
        panic!("click_result_rows should parse");
    };
    assert!(states.matches(true, false) && !states.matches(false, true));
    assert_eq!(modifiers, vec![Modifier::Ctrl]);
    assert!(
        parse_command(r#"{"click_result_rows":{}}"#)
            .expect_err("an empty state should be rejected")
            .contains("needs a state")
    );
    assert!(
        predicate(json!({ "any_result_row": { "focused": true } }))
            .check
            .is_live()
    );
}

#[test]
fn prefs_panes_clipboard_holds_and_notes_parse_only_in_their_valid_shapes() {
    assert!(
        serde_json::from_value::<Predicate>(
            json!({"pref": {"key": "list_hidden_columns", "contains": "size"}})
        )
        .is_ok()
    );
    assert!(
        serde_json::from_value::<Predicate>(json!({"pref": {"key": "k", "value": null}})).is_ok()
    );
    for bad in [
        json!({"pref": {"key": "k"}}),
        json!({"pref": {"key": "k", "value": 1, "contains": "x"}}),
        json!({"panes_with": {"target": {"entry": "a"}, "names": []}}),
        json!({"panes_with": {"target": {"role": "button", "within": {"preview": true}}, "names": []}}),
        json!({"present": {"role": "label", "name_matches": "("}}),
        json!({"present": {"role": "label", "states": ["active"]}}),
    ] {
        assert!(
            serde_json::from_value::<Predicate>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    assert_eq!(
        parse_command(r#"{"clipboard":{"starts_with":"alpha","longer_than":5},"message":"m"}"#),
        Ok(Command::Clipboard {
            text: serde_json::from_value(json!({"starts_with": "alpha", "longer_than": 5}))
                .expect("the json should parse"),
            message: Some("m".to_owned()),
        })
    );
    assert_eq!(
        parse_command(r#"{"clipboard_set":"selection-cleared"}"#),
        Ok(Command::ClipboardSet("selection-cleared".to_owned()))
    );
    assert!(parse_command(r#"{"expect":{"focused":null},"hold_ms":400}"#).is_err());
    assert!(
        parse_command(r#"{"expect":{"focused":null},"timeout_ms":3000,"hold_ms":400}"#).is_ok()
    );
    assert!(parse_command(r#"{"note":"number","as":"n"}"#).is_err());
    assert!(parse_command(r#"{"note":"entries","expr":1,"as":"n"}"#).is_err());
    assert!(
        parse_command(r#"{"note":"number","expr":{"of":{"window":true},"get":"width"},"as":"n"}"#)
            .is_ok()
    );
    assert!(parse_command(r#"{"note":"visible_top","pick":0,"as":"n"}"#).is_ok());
    assert!(parse_command(
        r#"{"note":"number","expr":{"pref":"w"},"until":[{"op":">","than":300}],"until_found":true,"as":"n"}"#
    )
    .is_ok());
    assert!(
        parse_command(
            r#"{"note":"number","expr":{"pref":"w"},"until":[{"op":">","than":300}],"as":"n"}"#
        )
        .is_err(),
        "until needs until_found"
    );
    let longer: TextMatch =
        serde_json::from_value(json!({"starts_with": "alpha", "longer_than": 5}))
            .expect("the json should parse");
    assert!(longer.matches("alpha bravo"));
    assert!(!longer.matches("alpha"));
    assert!(!longer.matches("bravo alpha"));
}

#[test]
fn batch_six_commands_parse_only_in_their_valid_shapes() {
    let target: Target =
        serde_json::from_value(json!({"role": ["check box", "toggle button"], "name": "Confirm"}))
            .expect("a role list should parse");
    assert_eq!(
        target.to_json(),
        json!({"role": ["check box", "toggle button"], "name": "Confirm"})
    );
    assert_eq!(
        serde_json::from_value::<Target>(json!({"other_window": "Strata"})).ok(),
        Some(Target::OtherWindow("Strata".to_owned()))
    );
    assert!(serde_json::from_value::<Target>(json!({"role": [], "name": "x"})).is_err());
    let Ok(Command::Wait {
        never: Some(never), ..
    }) = parse_command(r#"{"wait":{"preview":true},"never":{"preview":null},"never_message":"m"}"#)
    else {
        panic!("a wait with never should parse");
    };
    assert_eq!(never.1.as_deref(), Some("m"));
    let Ok(Command::During { checks, .. }) = parse_command(
        r#"{"during":[{"check":{"preview":true},"message":"a"},{"check":{"peek":false}}],"ms":100}"#,
    ) else {
        panic!("a during list should parse");
    };
    assert_eq!(checks.len(), 2);
    for invalid in [
        r#"{"wait":{"preview":true},"never_message":"m"}"#,
        r#"{"during":[],"ms":100}"#,
        r#"{"during":[{"check":{"peek":false}}],"ms":100,"message":"m"}"#,
        r#"{"wait":{"largest_image":{"within":{"preview":true},"size":[1,1],"exceeds":[1,1]}}}"#,
        r#"{"note":"entries","as":"n","until_names":{"at_least":2}}"#,
        r#"{"wait":{"pref":{"key":"k","value":1,"one_of":[1]}}}"#,
        r#"{"wait":{"mentions":{"target":{"dialog":true},"text":"a","equals":"a"}}}"#,
    ] {
        assert!(
            parse_command(invalid).is_err(),
            "{invalid} should be rejected"
        );
    }
}

#[test]
fn portal_reveal_and_window_steps_parse_and_reject_what_they_cannot_mean() {
    assert_eq!(
        parse_command(r#"{"adopt":"Picker","timeout_ms":20000,"message":"m"}"#),
        Ok(Command::Adopt {
            title: "Picker".to_owned(),
            timeout: Duration::from_millis(20_000),
            message: Some("m".to_owned()),
        })
    );
    assert_eq!(
        parse_command(r#"{"keep":[1,2,3,4],"as":"foreign"}"#),
        Ok(Command::Keep {
            rect: [1, 2, 3, 4],
            name: "foreign".to_owned(),
        })
    );
    assert!(matches!(
        parse_command(
            r#"{"keys_until":{"focused":true},"keys":["Tab"],"times":3,"check_first":true,"await_focus_change":{"timeout_ms":5000},"exhausted":"never"}"#
        ),
        Ok(Command::KeysUntil {
            await_focus_change: Some((timeout, None)),
            exhausted: Some(_),
            ..
        }) if timeout == Duration::from_millis(5000)
    ));
    assert!(matches!(
        parse_command(r#"{"open_menu":{"role":"button","name":"A"},"until":{"menu_items":{}}}"#),
        Ok(Command::OpenMenu { open_states, .. }) if open_states == [StateName::Checked]
    ));
    for invalid in [
        r#"{"adopt":""}"#,
        r#"{"keep":[1,2,3],"as":"x"}"#,
        r#"{"reveal":{"role":"button"}}"#,
        r#"{"reveal_control":{"entry":"a"}}"#,
        r#"{"open_menu":{"role":"button","name":"A"},"until":{"menu_items":{}},"open_states":[]}"#,
        r#"{"wait":{"file":{"path":"/a","exists":true,"mode_bits":{"mask":"100","set":true}}}}"#,
        r#"{"wait":{"file":{"path":"/a","mode_bits":{"mask":"9","set":true}}}}"#,
        r#"{"wait":{"present":{"role":["*","button"]}}}"#,
        r#"{"wait":{"present":{"role":"list item","has":{"entry":"a"}}}}"#,
    ] {
        assert!(
            parse_command(invalid).is_err(),
            "{invalid} should be rejected"
        );
    }
    for valid in [
        r#"{"wait":{"some_window":{"title":"Strata","holds":{"windows":{"title":"Strata","count":2}}}}}"#,
        r#"{"wait":{"child_names":{"of":{"parent":{"sidebar":"Home"}},"role":"button","names":{"starts_with":["Home"]}}}}"#,
        r#"{"wait":{"present":{"role":"*","states":["selected"],"within":{"preview":true}}}}"#,
        r#"{"wait":{"present":{"role":"list item","has":{"role":"label","name":"a"}}}}"#,
        r#"{"wait":{"state":{"target":{"row":"a"},"focusable":true}}}"#,
        r#"{"expect":[{"alive":"x"},{"focus_outside":{"remembered":"x"}},{"focused_widget_is":{"remembered":"x"}}]}"#,
        r#"{"expect":{"subtree_text":{"target":{"dialog":true},"contains":"a","strip_newlines":true}}}"#,
    ] {
        assert!(parse_command(valid).is_ok(), "{valid} should parse");
    }
}
