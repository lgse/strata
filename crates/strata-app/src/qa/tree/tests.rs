// SPDX-License-Identifier: MIT

use serde_json::json;
use strata_qa::{Mark, Rect};

use super::{Drawn, DrawnButton, DrawnColumn, DrawnRow, NameLabel, Shape, Snapshot};

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

/// A scroll view as Rune's tree reports it: its box, its offsets and how far it scrolls down,
/// and its content's box.
fn scroll_view(scroll: [f64; 3], own: [f64; 4], content: [f64; 4]) -> serde_json::Value {
    json!({
        "role": "window",
        "children": [{
            "role": "scroll-view",
            "scroll": scroll,
            "bounds": own,
            "children": [{ "role": "group", "bounds": content }],
        }],
    })
}

#[test]
fn a_scroll_views_offsets_limits_and_bar_are_read_from_the_tree() {
    use strata_qa::{Orientation::Horizontal, Orientation::Vertical, Scroll};
    let scroll = |value: f64, page: f64, scrollable: bool| Scroll {
        value,
        page,
        scrollable,
    };
    // What Rune reports, then the scroll along x and along y, and the bar's position.
    let cases = [
        (
            "the Columns strip, scrolled to its end",
            scroll_view(
                [1401.0, 0.0, 0.0],
                [201.0, 36.0, 999.0, 724.0],
                [-1200.0, 36.0, 2400.0, 724.0],
            ),
            (scroll(1401.0, 999.0, true), scroll(0.0, 724.0, false)),
            Some(1401.0),
        ),
        (
            "a column's rows, scrolled down: the content's box is only the mounted rows",
            scroll_view(
                [0.0, 1397.0, 1397.0],
                [900.0, 77.0, 300.0, 683.0],
                [900.0, -1294.0, 300.0, 2054.0],
            ),
            (scroll(0.0, 300.0, false), scroll(1397.0, 683.0, true)),
            Some(1397.0),
        ),
        (
            "a strip at its start, wider than it shows",
            scroll_view(
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 400.0, 300.0],
                [0.0, 0.0, 900.0, 300.0],
            ),
            (scroll(0.0, 400.0, true), scroll(0.0, 300.0, false)),
            Some(0.0),
        ),
        (
            "the sidebar, whose places fit",
            scroll_view(
                [0.0, 0.0, 0.0],
                [0.0, 36.0, 201.0, 724.0],
                [0.0, 36.0, 201.0, 136.0],
            ),
            (scroll(0.0, 201.0, false), scroll(0.0, 724.0, false)),
            None,
        ),
    ];
    for (case, tree, (across, down), bar) in cases {
        let snapshot = Snapshot::read(&tree, &Drawn::default());
        let view = snapshot.at(1).expect("the scroll view");
        let scrolled = view.scroll.expect("a scroll view's offsets");
        assert_eq!(
            (scrolled.along(Horizontal), scrolled.along(Vertical)),
            (across, down),
            "{case}"
        );
        assert_eq!(scrolled.bar(), bar, "{case}");
    }

    let plain =
        json!({ "role": "window", "children": [{ "role": "group", "bounds": [0, 0, 10, 10] }] });
    let snapshot = Snapshot::read(&plain, &Drawn::default());
    assert_eq!(
        snapshot.at(1).map(|seen| seen.scroll),
        Some(None),
        "a group scrolls nothing"
    );
}

/// `keyboard_navigation/back-history-dropdown` lets go of a press held on Back after its
/// history opened, when Rune's tree holds the menu alone: the button is presented from what
/// is drawn, once, and a button the tree has is not presented again. With no menu open,
/// a dialog's modal included, nothing is added.
#[test]
fn a_button_behind_an_open_menu_is_presented_from_what_is_drawn() {
    let tree = json!({
        "role": "window",
        "children": [
            { "role": "menu", "name": "Context menu", "states": ["modal"], "children": [
                { "role": "menu-item", "key": "pane-history-item2", "name": "documents" },
            ] },
            { "role": "button", "key": "header-search", "name": "Search" },
        ],
    });
    let button = |key: &str, name: &str| DrawnButton {
        key: key.to_owned(),
        name: name.to_owned(),
        bounds: bounds(0.0),
    };
    let drawn = Drawn {
        buttons: vec![
            button("icons-back", "Back (Alt+←)"),
            button("header-search", "Search"),
        ],
        ..Drawn::default()
    };

    let snapshot = Snapshot::read(&tree, &drawn);

    let buttons: Vec<_> = (0..)
        .map_while(|index| snapshot.at(index))
        .filter(|seen| seen.role() == "button")
        .map(|seen| (seen.key.as_deref(), seen.name.as_str(), seen.bounds))
        .collect();
    assert_eq!(
        buttons,
        [
            (Some("header-search"), "Search", None),
            (Some("icons-back"), "Back (Alt+←)", Some(bounds(0.0))),
        ]
    );

    let open = json!({
        "role": "window",
        "children": [{ "role": "button", "key": "header-search", "name": "Search" }],
    });
    let snapshot = Snapshot::read(&open, &drawn);
    let named: Vec<_> = (0..)
        .map_while(|index| snapshot.at(index))
        .filter(|seen| seen.role() == "button")
        .map(|seen| seen.name.as_str())
        .collect();
    assert_eq!(named, ["Search"], "nothing is added with no modal open");

    let dialog = json!({
        "role": "window",
        "children": [{ "role": "dialog", "name": "Unable to open location", "states": ["modal"] }],
    });
    let snapshot = Snapshot::read(&dialog, &drawn);
    let behind = (0..)
        .map_while(|index| snapshot.at(index))
        .any(|seen| seen.role() == "button");
    assert!(
        !behind,
        "nothing is presented behind a dialog, to stay cautious"
    );
}
