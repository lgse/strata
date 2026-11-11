// SPDX-License-Identifier: MIT

use serde_json::json;
use strata_qa::{Mark, Rect};

use super::{NameLabel, Snapshot};

fn label(above: &[&str], text: &str, x: f64) -> NameLabel {
    NameLabel {
        above: above.iter().map(|key| (*key).to_owned()).collect(),
        text: text.to_owned(),
        bounds: Rect {
            x,
            y: 80.0,
            width: 50.0,
            height: 18.0,
        },
    }
}

#[test]
fn a_drawn_name_is_a_label_under_its_entry_and_under_nothing_else() {
    let tree = json!({
        "role": "window",
        "children": [
            { "role": "button", "key": "row-9", "name": "not an entry" },
            { "role": "group", "name": "folder", "children": [
                { "role": "list-box", "key": "columns-col-1", "children": [
                    { "role": "list-box-option", "key": "row-1", "name": "archive" },
                    { "role": "list-box-option", "key": "row-2", "name": "pictures" },
                ] },
            ] },
        ],
    });
    let labels = [
        label(&["content", "row-2", "rows", "columns-col-1"], "pictures", 30.0),
        // A label keyed like a row's under something that is no entry, and one under no row.
        label(&["row-9"], "not an entry", 0.0),
        label(&["columns-col-1-title"], "folder", 0.0),
    ];

    let snapshot = Snapshot::read(&tree, &["columns-col-1".to_owned()], &labels);

    let presented: Vec<_> = (0..)
        .map_while(|index| snapshot.at(index))
        .filter(|seen| seen.role == "label")
        .collect();
    assert_eq!(presented.len(), 1, "{presented:?}");
    let name = presented[0];
    assert_eq!((name.role(), name.name.as_str()), ("label", "pictures"));
    assert_eq!(name.bounds, Some(labels[0].bounds));
    let row = snapshot.at(name.parent.expect("a row")).expect("the row");
    assert_eq!((row.mark, row.name.as_str()), (Some(Mark::Entry), "pictures"));
    let children: Vec<_> = row.children.iter().map(|child| snapshot.at(*child)).collect();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].and_then(|seen| seen.node.clone()), name.node);
}
