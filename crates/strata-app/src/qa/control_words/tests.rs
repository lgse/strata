// SPDX-License-Identifier: MIT

use rune::ElementInfo;
use serde_json::json;

use super::super::tree::{Drawn, Snapshot};
use super::Words;

fn element(parent: Option<usize>, role: &'static str, key: Option<&str>, text: Option<&str>, rect: [f32; 4]) -> ElementInfo {
    ElementInfo {
        parent,
        role,
        key: key.map(str::to_owned),
        text: text.map(str::to_owned),
        rect,
        ..ElementInfo::default()
    }
}

/// The labels the snapshot holds under the node keyed `key`, by name, in order.
fn labels_under(snapshot: &Snapshot, key: &str) -> Vec<String> {
    let Some(control) = (0..)
        .map_while(|index| snapshot.at(index))
        .find(|seen| seen.key.as_deref() == Some(key))
    else {
        return Vec::new();
    };
    control
        .children
        .iter()
        .filter_map(|child| snapshot.at(*child))
        .filter(|seen| seen.role == "label")
        .map(|seen| seen.name.clone())
        .collect()
}

/// A menu item says its text; a search option says its name and folder, in the order drawn; a
/// shortcut its name does not say stays out, as does a text inside a button in an option.
#[test]
fn presents_the_texts_a_one_leaf_control_says_under_it() {
    let row = [0.0, 0.0, 200.0, 28.0];
    let elements = vec![
        element(None, "", Some("root"), None, [0.0, 0.0, 800.0, 600.0]),
        element(Some(0), "menuitemradio", Some("opt-6"), None, row),
        element(Some(1), "", None, Some("日本語"), [8.0, 4.0, 40.0, 18.0]),
        element(Some(0), "option", Some("result-0"), None, [0.0, 40.0, 400.0, 50.0]),
        element(Some(3), "", None, Some("summary.md"), [8.0, 44.0, 80.0, 18.0]),
        element(Some(3), "", None, Some("/home/reports"), [8.0, 64.0, 120.0, 18.0]),
        element(Some(0), "menuitem", Some("cut"), None, [0.0, 100.0, 200.0, 28.0]),
        element(Some(6), "", None, Some("Cut"), [8.0, 104.0, 30.0, 18.0]),
        element(Some(6), "", None, Some("Ctrl+X"), [150.0, 104.0, 40.0, 18.0]),
        element(Some(0), "option", Some("result-1"), None, [0.0, 140.0, 400.0, 50.0]),
        element(Some(9), "button", Some("reveal"), None, [300.0, 144.0, 80.0, 20.0]),
        element(Some(10), "", None, Some("Reveal"), [304.0, 146.0, 60.0, 16.0]),
    ];
    let tree = json!({ "role": "window", "children": [
        { "role": "menu-item-radio", "key": "opt-6", "name": "日本語", "bounds": [0.0, 0.0, 200.0, 28.0] },
        { "role": "list-box-option", "key": "result-0", "name": "/home/reports/summary.md", "bounds": [0.0, 40.0, 400.0, 50.0] },
        { "role": "menu-item", "key": "cut", "name": "Cut", "bounds": [0.0, 100.0, 200.0, 28.0] },
        { "role": "list-box-option", "key": "result-1", "name": "Reveal elsewhere", "bounds": [0.0, 140.0, 400.0, 50.0] },
    ] });
    let drawn = Drawn {
        words: Words::drawn(&elements),
        ..Drawn::default()
    };
    let snapshot = Snapshot::read(&tree, &drawn);

    let cases = [
        ("opt-6", vec!["日本語"]),
        ("result-0", vec!["summary.md", "/home/reports"]),
        ("cut", vec!["Cut"]),
        ("result-1", vec![]),
    ];
    for (key, expected) in cases {
        assert_eq!(labels_under(&snapshot, key), expected, "under {key}");
    }
}

/// A control the snapshot already holds children under, such as an entry row with its
/// presented name, gets no second label.
#[test]
fn leaves_a_control_with_children_as_it_is() {
    let elements = vec![
        element(None, "", Some("root"), None, [0.0, 0.0, 800.0, 600.0]),
        element(Some(0), "option", Some("row-1"), None, [0.0, 0.0, 300.0, 26.0]),
        element(Some(1), "", None, Some("readme.md"), [8.0, 4.0, 80.0, 18.0]),
    ];
    let tree = json!({ "role": "window", "children": [
        { "role": "list-box-option", "key": "row-1", "name": "readme.md", "bounds": [0.0, 0.0, 300.0, 26.0], "children": [
            { "role": "label", "name": "readme.md", "bounds": [8.0, 4.0, 80.0, 18.0] },
        ] },
    ] });
    let drawn = Drawn {
        words: Words::drawn(&elements),
        ..Drawn::default()
    };
    let snapshot = Snapshot::read(&tree, &drawn);

    assert_eq!(labels_under(&snapshot, "row-1"), vec!["readme.md"]);
}
