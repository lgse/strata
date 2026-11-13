// SPDX-License-Identifier: MIT

use serde_json::json;
use strata_qa::{Mark, Rect};

use super::{Drawn, DrawnColumn, DrawnRow, NameLabel, Shape, Snapshot};

/// The Columns view's shape, as its probe states it.
const COLUMNS: Shape = Shape {
    name: "Columns",
    list: "list-box",
    entry: "list-box-option",
};

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
        label(
            &["content", "row-2", "rows", "columns-col-1"],
            "pictures",
            30.0,
        ),
        // A label keyed like a row's under something that is no entry, and one under no row.
        label(&["row-9"], "not an entry", 0.0),
        label(&["columns-col-1-title"], "folder", 0.0),
    ];

    let drawn = Drawn {
        view: Some(COLUMNS),
        lists: vec!["columns-col-1".to_owned()],
        columns: Vec::new(),
        labels: labels.to_vec(),
        ..Drawn::default()
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
    assert_eq!(
        (row.mark, row.name.as_str()),
        (Some(Mark::Entry), "pictures")
    );
    let children: Vec<_> = row
        .children
        .iter()
        .map(|child| snapshot.at(*child))
        .collect();
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

/// One column as drawn: `folder`, with `readme.md` and `todo.txt`, the second selected.
fn drawn_folder() -> Drawn {
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
        rows: vec![
            row("row-1", "readme.md", false),
            row("row-2", "todo.txt", true),
        ],
    };
    Drawn {
        view: Some(COLUMNS),
        lists: vec![column.list.clone()],
        columns: vec![column],
        labels: Vec::new(),
        ..Drawn::default()
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
    let drawn = drawn_folder();

    let snapshot = Snapshot::read(&menu, &drawn);

    let seen: Vec<_> = (0..).map_while(|index| snapshot.at(index)).collect();
    let marked = |mark: Mark| seen.iter().filter(move |seen| seen.mark == Some(mark));
    let pane = marked(Mark::Pane("Columns")).next().expect("the pane");
    assert_eq!((pane.role(), pane.name.as_str()), ("grouping", "folder"));
    let list = marked(Mark::EntryContainer).next().expect("the list");
    let around = list.parent.and_then(|index| snapshot.at(index));
    assert_eq!(list.role(), "list");
    assert_eq!(
        around.map(|around| around.node.clone()),
        Some(pane.node.clone())
    );
    let rows: Vec<_> = marked(Mark::Entry)
        .map(|row| (row.role(), row.name.as_str(), row.has_state("selected")))
        .collect();
    assert_eq!(
        rows,
        [
            ("list item", "readme.md", false),
            ("list item", "todo.txt", true)
        ]
    );
    // A menu row's shortcut reads as its description, unless it has one of its own.
    let described: Vec<_> = seen
        .iter()
        .filter(|seen| seen.role() == "menu item")
        .map(|seen| (seen.name.as_str(), seen.description.as_str()))
        .collect();
    assert_eq!(
        described,
        [
            ("Copy", "Ctrl+C"),
            ("Open With…", "Looking for compatible applications…")
        ]
    );
}

#[test]
fn a_column_the_tree_holds_is_not_presented_twice() {
    let held = json!({ "role": "window", "children": [
        { "role": "group", "name": "folder", "children": [
            { "role": "list-box", "key": "columns-col-1", "children": [] } ] } ] });

    let once = Snapshot::read(&held, &drawn_folder());

    let lists = (0..).map_while(|index| once.at(index));
    assert_eq!(lists.filter(|seen| seen.role == "list-box").count(), 1);
}

/// The List view's shape, as its probe states it.
const LIST: Shape = Shape {
    name: "List",
    list: "grid",
    entry: "row",
};

/// The names of the entries a snapshot of `tree` reads, with `list` the List table's key.
fn entries_of(tree: &serde_json::Value, list: &str) -> Vec<String> {
    let drawn = Drawn {
        view: Some(LIST),
        lists: vec![list.to_owned()],
        columns: Vec::new(),
        labels: Vec::new(),
        ..Drawn::default()
    };
    let snapshot = Snapshot::read(tree, &drawn);
    (0..)
        .map_while(|index| snapshot.at(index))
        .filter(|seen| seen.mark == Some(Mark::Entry))
        .map(|seen| seen.name.clone())
        .collect()
}

/// The table's headings are a row of its grid, as C4 builds it, and no entry.
#[test]
fn a_tables_headings_row_is_not_an_entry() {
    let tree = json!({ "role": "window", "children": [
        { "role": "group", "name": "folder", "children": [
            { "role": "grid", "key": "list", "name": "folder", "children": [
                { "role": "group", "key": "list-headings-menu-trigger", "children": [
                    { "role": "row", "key": "list-headings", "name": "Name", "children": [
                        { "role": "column-header", "key": "head-name", "name": "Name" } ] } ] },
                { "role": "scroll-view", "key": "viewport", "children": [
                    { "role": "group", "key": "rows", "children": [
                        { "role": "row", "key": "row-1", "name": "archive" },
                        { "role": "row", "key": "row-2", "name": "readme.md" } ] } ] } ] } ] } ] });

    assert_eq!(entries_of(&tree, "list"), ["archive", "readme.md"]);
}

/// A group's header line in a grouped list is no entry either, whatever its role: it is keyed
/// as a header (`header_key`), not as a row.
#[test]
fn a_groups_header_is_not_an_entry() {
    let tree = json!({ "role": "window", "children": [
        { "role": "group", "name": "folder", "children": [
            { "role": "grid", "key": "list", "name": "folder", "children": [
                { "role": "group", "key": "rows", "children": [
                    { "role": "row", "key": "head-8322393941", "name": "Folder" },
                    { "role": "row", "key": "row-1", "name": "archive" },
                    { "role": "heading", "key": "head-52759", "name": "Text" },
                    { "role": "row", "key": "row-4", "name": "todo.txt" } ] } ] } ] } ] });

    assert_eq!(entries_of(&tree, "list"), ["archive", "todo.txt"]);
}
