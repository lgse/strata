// SPDX-License-Identifier: MIT

use serde_json::json;
use strata_qa::{Mark, Rect};

use super::{Drawn, DrawnColumn, DrawnRow, NameLabel, Snapshot};

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

    let drawn = Drawn {
        lists: vec!["columns-col-1".to_owned()],
        columns: Vec::new(),
        labels: labels.to_vec(),
    };
    let snapshot = Snapshot::read(&tree, &drawn);

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

fn bounds(x: f64) -> Rect {
    Rect {
        x,
        y: 40.0,
        width: 300.0,
        height: 26.0,
    }
}

/// `selection/right-click-selects-entry` waits for the pane's selection with the entry's
/// menu open, when Rune's tree holds the menu alone.
#[test]
fn a_column_behind_an_open_menu_is_presented_from_what_is_drawn() {
    let menu = json!({
        "role": "window",
        "children": [
            { "role": "menu", "name": "todo.txt", "states": ["modal"], "children": [
                { "role": "menu-item", "key": "menu-item3", "name": "Copy", "shortcut": "Ctrl+C" },
                { "role": "menu-item", "key": "menu-item4", "name": "Open With…",
                  "description": "Looking for compatible applications…", "shortcut": "" },
            ] },
        ],
    });
    let row = |key: &str, name: &str, selected: bool| DrawnRow {
        key: key.to_owned(),
        name: name.to_owned(),
        selected,
        bounds: bounds(0.0),
    };
    let column = DrawnColumn {
        list: "columns-col-1".to_owned(),
        name: "folder".to_owned(),
        bounds: bounds(0.0),
        pane: bounds(0.0),
        rows: vec![row("row-1", "readme.md", false), row("row-2", "todo.txt", true)],
    };
    let drawn = Drawn {
        lists: vec![column.list.clone()],
        columns: vec![column],
        labels: Vec::new(),
    };

    let snapshot = Snapshot::read(&menu, &drawn);

    let seen: Vec<_> = (0..).map_while(|index| snapshot.at(index)).collect();
    let marked = |mark: Mark| seen.iter().filter(move |seen| seen.mark == Some(mark));
    let pane = marked(Mark::Pane("Columns")).next().expect("the pane");
    assert_eq!((pane.role(), pane.name.as_str()), ("grouping", "folder"));
    let list = marked(Mark::EntryContainer).next().expect("the list");
    let around = list.parent.and_then(|index| snapshot.at(index));
    assert_eq!(list.role(), "list");
    assert_eq!(around.map(|around| around.node.clone()), Some(pane.node.clone()));
    let rows: Vec<_> = marked(Mark::Entry)
        .map(|row| (row.role(), row.name.as_str(), row.has_state("selected")))
        .collect();
    assert_eq!(
        rows,
        [("list item", "readme.md", false), ("list item", "todo.txt", true)]
    );
    // A menu row's shortcut reads as its description, unless it has one of its own.
    let described: Vec<_> = seen
        .iter()
        .filter(|seen| seen.role() == "menu item")
        .map(|seen| (seen.name.as_str(), seen.description.as_str()))
        .collect();
    assert_eq!(
        described,
        [("Copy", "Ctrl+C"), ("Open With…", "Looking for compatible applications…")]
    );

    // A column the tree does hold is not presented twice.
    let held = json!({ "role": "window", "children": [
        { "role": "group", "name": "folder", "children": [
            { "role": "list-box", "key": "columns-col-1", "children": [] } ] } ] });
    let once = Snapshot::read(&held, &drawn);
    let lists = (0..).map_while(|index| once.at(index));
    assert_eq!(lists.filter(|seen| seen.role == "list-box").count(), 1);
}
