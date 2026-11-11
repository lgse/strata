use serde_json::json;

use crate::protocol::{
    Command, Event, Message, Payload, Request, Selection, Via, chord_keys, fnv1a64, pattern,
};

#[test]
fn a_request_reads_from_its_wire_form() {
    let line = r#"{"id":3,"cmd":"selection-read","selection":"primary","via":"data-control","mime":"text/plain"}"#;
    let req: Request = serde_json::from_str(line).expect("request");
    assert_eq!(
        req,
        Request {
            id: 3,
            command: Command::SelectionRead {
                selection: Selection::Primary,
                via: Via::DataControl,
                mime: "text/plain".into(),
            },
        }
    );
}

#[test]
fn a_key_command_without_an_edge_is_a_chord() {
    let req: Request = serde_json::from_str(r#"{"id":1,"cmd":"key","key":"ctrl+c"}"#).unwrap();
    assert_eq!(
        req.command,
        Command::Key {
            key: "ctrl+c".into(),
            pressed: None
        }
    );
}

#[test]
fn replies_and_events_tell_themselves_apart() {
    let reply: Message = serde_json::from_value(json!({"id": 7, "ok": true})).unwrap();
    assert!(matches!(reply, Message::Reply(r) if r.id == 7 && r.ok));
    let event: Message =
        serde_json::from_value(json!({"event": "focus", "focused": true})).unwrap();
    assert_eq!(event, Message::Event(Event::Focus { focused: true }));
}

#[test]
fn a_pattern_is_the_same_for_a_seed_and_differs_across_seeds() {
    let a = pattern(1000, 1);
    assert_eq!(a.len(), 1000);
    assert_eq!(a, pattern(1000, 1));
    assert_ne!(a, pattern(1000, 2));
    // A shorter pattern is a prefix of a longer one, so lengths that aren't multiples of 8
    // still agree byte for byte.
    assert_eq!(pattern(13, 5), pattern(16, 5)[..13]);
}

#[test]
fn fnv1a64_matches_the_reference_vectors() {
    assert_eq!(fnv1a64(b""), "cbf29ce484222325");
    assert_eq!(fnv1a64(b"a"), "af63dc4c8601ec8c");
    assert_eq!(fnv1a64(b"foobar"), "85944171f73967e8");
}

#[test]
fn a_payload_reads_its_bytes() {
    assert_eq!(Payload::Text("hé".into()).bytes().unwrap(), "hé".as_bytes());
    let p = Payload::Pattern { len: 9, seed: 3 };
    assert_eq!(p.bytes().unwrap(), pattern(9, 3));
}

#[test]
fn a_chord_lists_its_modifiers_first() {
    assert_eq!(
        chord_keys("ctrl+Shift+a").unwrap(),
        ["Control_L", "Shift_L", "a"]
    );
    assert_eq!(chord_keys("Return").unwrap(), ["Return"]);
    assert_eq!(chord_keys("shift").unwrap(), ["Shift_L"]);
    assert_eq!(chord_keys("ctrl+alt").unwrap(), ["Control_L", "Alt_L"]);
    assert!(chord_keys("hyper+a").is_err());
    assert!(chord_keys("ctrl+").is_err());
}
