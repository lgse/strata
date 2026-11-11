// SPDX-License-Identifier: MIT

use std::{fs::File, future::Future, io::Read as _, pin::pin, task};

use serde_json::{Value, json};

use super::{
    super::{Mark, serve},
    fake::{Fake, Node, WINDOW, drawn},
};

/// Runs a session over the lines and returns the replies it wrote.
fn session(fake: &Fake, name: &str, lines: &[Value]) -> Vec<Value> {
    let path = std::env::temp_dir().join(format!("strata-qa-{}-{name}", std::process::id()));
    let protocol = File::create(&path).expect("a file for the replies");
    let (sender, commands) = futures_channel::mpsc::unbounded();
    for line in lines {
        sender
            .unbounded_send(line.to_string())
            .expect("the session is listening");
    }
    drop(sender);
    let mut context = task::Context::from_waker(task::Waker::noop());
    assert!(
        pin!(serve(fake, protocol, commands))
            .poll(&mut context)
            .is_ready(),
        "the session ends when its input does"
    );
    let mut written = String::new();
    File::open(&path)
        .and_then(|mut file| file.read_to_string(&mut written))
        .expect("the replies can be read back");
    let _ = std::fs::remove_file(&path);
    written
        .lines()
        .map(|line| serde_json::from_str(line).expect("each reply is one JSON line"))
        .collect()
}

#[test]
fn a_session_answers_ready_then_each_line_and_quits_when_input_ends() {
    let mut fake = Fake::new();
    let pane = fake.add(
        WINDOW,
        Node {
            mark: Some(Mark::Pane("Columns")),
            ..drawn("grouping", "home", [0.0, 0.0, 400.0, 600.0])
        },
    );
    let list = fake.add(
        pane,
        Node {
            mark: Some(Mark::EntryContainer),
            ..drawn("list", "home", [0.0, 40.0, 400.0, 560.0])
        },
    );
    fake.add(
        list,
        Node {
            mark: Some(Mark::Entry),
            ..drawn("list item", "notes", [0.0, 40.0, 400.0, 30.0])
        },
    );

    let replies = session(
        &fake,
        "answers",
        &[
            json!({ "expect": { "entries": ["notes"] } }),
            json!({ "expect": { "entries": ["other"] }, "message": "the other listing" }),
            json!({ "no_such_step": true }),
            json!({ "query": "state" }),
        ],
    );
    assert_eq!(
        replies.len(),
        5,
        "one reply for ready and one for each line"
    );

    let ready = &replies[0];
    assert_eq!(ready["result"], json!({ "ready": true }));
    assert_eq!(ready["app"]["directory"], json!("home"));
    assert_eq!(ready["app"]["view_mode"], json!("Columns"));
    assert!(
        ready["changed"]
            .as_array()
            .is_some_and(|keys| keys.contains(&json!("panes"))),
        "everything is new in the first reply"
    );

    assert_eq!(replies[1]["ok"], json!(true));
    assert_eq!(
        replies[1]["changed"],
        json!([]),
        "nothing moved between the two replies"
    );

    assert_eq!(replies[2]["ok"], json!(false));
    assert_eq!(
        replies[2]["error"],
        Value::Null,
        "an expectation that fails is no error"
    );
    assert_eq!(replies[2]["failures"][0]["actual"], json!(["notes"]));
    assert_eq!(
        replies[2]["failures"][0]["message"],
        json!("the other listing")
    );

    assert_eq!(replies[3]["ok"], json!(false));
    assert!(
        replies[3]["error"].is_string(),
        "a line that does not parse is answered"
    );

    assert_eq!(replies[4]["result"], replies[4]["app"]);
    assert_eq!(fake.sent.borrow().last().map(String::as_str), Some("quit"));
}
