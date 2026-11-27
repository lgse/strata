// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use serde_json::json;
use strata_components::contract::EntryId;
use strata_qa::{EntryPart, Mark, Rect};

use super::{Drawn, EntryParts, Shape, Snapshot};

mod pdf;

/// The Columns view's shape, as its probe states it.
const COLUMNS: Shape = Shape {
    name: "Columns",
    list: "list-box",
    entry: "list-box-option",
};

/// The List view's shape, as its probe states it.
const LIST: Shape = Shape {
    name: "List",
    list: "list",
    entry: "list-item",
};

fn bounds(x: f64) -> Rect {
    Rect {
        x,
        y: 40.0,
        width: 300.0,
        height: 26.0,
    }
}

#[test]
fn entry_part_frames_resolve_without_injecting_synthetic_tree_nodes() {
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
    let label_box = Rect {
        x: 30.0,
        y: 80.0,
        width: 50.0,
        height: 18.0,
    };
    let icon_box = bounds(4.0);
    let mut parts = HashMap::new();
    parts.insert(
        EntryId::new(2),
        EntryParts {
            label: Some(label_box),
            icon: Some(icon_box),
        },
    );
    parts.insert(
        EntryId::new(9),
        EntryParts {
            label: Some(bounds(0.0)),
            icon: Some(bounds(0.0)),
        },
    );

    let drawn = Drawn {
        view: Some(COLUMNS),
        lists: vec!["columns-col-1".to_owned()],
        parts,
        ..Drawn::default()
    };
    let snapshot = Snapshot::read(&tree, &drawn);

    assert!(
        (0..)
            .map_while(|i| snapshot.at(i))
            .all(|seen| !matches!(seen.role.as_str(), "label" | "image")),
        "no synthetic label or image nodes are injected into the accessibility tree"
    );

    let pictures = (0..)
        .map_while(|i| snapshot.at(i))
        .find(|s| s.key.as_deref() == Some("row-2"))
        .expect("pictures row");
    assert_eq!(pictures.mark, Some(Mark::Entry));
    assert!(pictures.children.is_empty());
    let pictures_node = pictures.node.as_ref().expect("node");
    assert_eq!(
        snapshot.entry_part_frame(pictures_node, EntryPart::Label),
        Some(label_box)
    );
    assert_eq!(
        snapshot.entry_part_frame(pictures_node, EntryPart::Icon),
        Some(icon_box)
    );
    assert_eq!(
        snapshot.entry_part_frame(pictures_node, EntryPart::Metadata),
        None
    );

    let button = (0..)
        .map_while(|i| snapshot.at(i))
        .find(|s| s.key.as_deref() == Some("row-9"))
        .expect("non-entry button");
    assert_eq!(
        snapshot.entry_part_frame(button.node.as_ref().expect("node"), EntryPart::Label),
        None,
        "non-entry nodes never expose entry parts"
    );
}

#[test]
fn merged_windows_keep_distinct_entry_parts_for_matching_entry_ids() {
    let window_tree = |name: &str| {
        json!({
            "role": "window",
            "children": [
                { "role": "group", "name": "folder", "children": [
                    { "role": "list-box", "key": "columns-col-1", "children": [
                        { "role": "list-box-option", "key": "row-1", "name": name },
                    ] },
                ] },
            ],
        })
    };
    let drawn_with = |label: Rect, icon: Rect| Drawn {
        view: Some(COLUMNS),
        lists: vec!["columns-col-1".to_owned()],
        parts: HashMap::from([(
            EntryId::new(1),
            EntryParts {
                label: Some(label),
                icon: Some(icon),
            },
        )]),
        ..Drawn::default()
    };
    let first_label = bounds(30.0);
    let first_icon = bounds(4.0);
    let second_label = bounds(430.0);
    let second_icon = bounds(404.0);

    let first = Snapshot::read_named(
        "window",
        &window_tree("first-window-row"),
        &drawn_with(first_label, first_icon),
    );
    let second = Snapshot::read_named(
        "window-2",
        &window_tree("second-window-row"),
        &drawn_with(second_label, second_icon),
    );
    let merged = first.with_window_behind(&second);

    let node_named = |name: &str| {
        (0..)
            .map_while(|i| merged.at(i))
            .find(|seen| seen.name == name)
            .and_then(|seen| seen.node.as_ref())
            .expect("merged row")
            .clone()
    };
    let first_node = node_named("first-window-row");
    let second_node = node_named("second-window-row");

    assert_eq!(
        merged.entry_part_frame(&first_node, EntryPart::Label),
        Some(first_label)
    );
    assert_eq!(
        merged.entry_part_frame(&first_node, EntryPart::Icon),
        Some(first_icon)
    );
    assert_eq!(
        merged.entry_part_frame(&second_node, EntryPart::Label),
        Some(second_label)
    );
    assert_eq!(
        merged.entry_part_frame(&second_node, EntryPart::Icon),
        Some(second_icon)
    );
}

#[test]
fn menu_row_shortcuts_read_as_descriptions_unless_already_described() {
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
    let snapshot = Snapshot::read(&menu, &Drawn::default());
    let described: Vec<_> = (0..)
        .map_while(|index| snapshot.at(index))
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

/// The table's headings, a box of buttons over the list as C4 builds it, are no entry.
#[test]
fn a_tables_headings_row_is_not_an_entry() {
    let tree = json!({ "role": "window", "children": [
        { "role": "group", "name": "folder", "children": [
            { "role": "list", "key": "list", "name": "folder", "children": [
                { "role": "group", "key": "list-headings-menu-trigger", "children": [
                    { "role": "group", "key": "list-headings", "children": [
                        { "role": "button", "key": "head-name", "name": "Name" } ] } ] },
                { "role": "scroll-view", "key": "viewport", "children": [
                    { "role": "group", "key": "rows", "children": [
                        { "role": "list-item", "key": "row-1", "name": "archive" },
                        { "role": "list-item", "key": "row-2", "name": "readme.md" } ] } ] } ] } ] } ] });

    assert_eq!(entries_of(&tree, "list"), ["archive", "readme.md"]);
}

#[test]
fn a_groups_header_is_not_an_entry() {
    let tree = json!({ "role": "window", "children": [
        { "role": "group", "name": "folder", "children": [
            { "role": "list", "key": "list", "name": "folder", "children": [
                { "role": "group", "key": "rows", "children": [
                    { "role": "list-item", "key": "head-8322393941", "name": "Folder" },
                    { "role": "list-item", "key": "row-1", "name": "archive" },
                    { "role": "heading", "key": "head-52759", "name": "Text" },
                    { "role": "list-item", "key": "row-4", "name": "todo.txt" } ] } ] } ] } ] });

    assert_eq!(entries_of(&tree, "list"), ["archive", "todo.txt"]);
}

fn scroll_view(
    scroll: [f64; 3],
    own: [f64; 4],
    actions: &[&str],
    content: [f64; 4],
) -> serde_json::Value {
    json!({
        "role": "window",
        "children": [{
            "role": "scroll-view",
            "scroll": scroll,
            "bounds": own,
            "actions": actions,
            "children": [{ "role": "group", "bounds": content }],
        }],
    })
}

const DOWN: &[&str] = &["scroll-up", "scroll-down", "set-scroll-offset"];
const SIDEWAYS: &[&str] = &[
    "scroll-up",
    "scroll-down",
    "scroll-left",
    "set-scroll-offset",
];

#[test]
fn a_scroll_views_offsets_limits_and_bar_are_read_from_the_tree() {
    use strata_qa::{Orientation::Horizontal, Orientation::Vertical, Scroll};
    let scroll = |value: f64, page: f64, scrollable: bool| Scroll {
        value,
        page,
        scrollable,
    };
    let cases = [
        (
            "the Columns strip, scrolled to its end",
            scroll_view(
                [1401.0, 0.0, 0.0],
                [201.0, 36.0, 999.0, 724.0],
                SIDEWAYS,
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
                DOWN,
                [900.0, -1294.0, 300.0, 2054.0],
            ),
            (scroll(0.0, 300.0, false), scroll(1397.0, 683.0, true)),
            Some(1397.0),
        ),
        (
            "a strip at its start",
            scroll_view(
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 400.0, 300.0],
                SIDEWAYS,
                [0.0, 0.0, 900.0, 300.0],
            ),
            (scroll(0.0, 400.0, true), scroll(0.0, 300.0, false)),
            Some(0.0),
        ),
        (
            "a view that scrolls only down, with a child wider than it",
            scroll_view(
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 400.0, 300.0],
                DOWN,
                [0.0, 0.0, 900.0, 200.0],
            ),
            (scroll(0.0, 400.0, false), scroll(0.0, 300.0, false)),
            None,
        ),
        (
            "the sidebar, whose places fit",
            scroll_view(
                [0.0, 0.0, 0.0],
                [0.0, 36.0, 201.0, 724.0],
                DOWN,
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

fn focusable_in(stop: Stop, names: &[&str]) -> Vec<bool> {
    let (strip, list) = match stop {
        Stop::List => (
            json!({}),
            json!({ "actions": ["focus"], "activeDescendant": "row-1" }),
        ),
        Stop::Strip => (
            json!({ "actions": ["focus"], "activeDescendant": "row-1" }),
            json!({}),
        ),
        Stop::NamingNone => (json!({}), json!({ "actions": ["focus"] })),
        Stop::None => (json!({}), json!({})),
    };
    let mut column = json!({ "role": "list-box", "key": "columns-col-1", "children": [
        { "role": "list-box-option", "key": "row-1", "name": "archive" },
        { "role": "label", "name": "not an entry" },
    ] });
    let mut around = json!({ "role": "group", "key": "columns-strip", "children": [
        { "role": "group", "name": "folder", "children": [] },
    ] });
    for (node, extra) in [(&mut column, list), (&mut around, strip)] {
        if let Some(extra) = extra.as_object() {
            node.as_object_mut().expect("a node").extend(extra.clone());
        }
    }
    around["children"][0]["children"] = json!([column]);
    let tree = json!({
        "role": "window",
        "children": [around, { "role": "list-box-option", "key": "row-2", "name": "outside" }],
    });
    let drawn = Drawn {
        view: Some(COLUMNS),
        lists: vec!["columns-col-1".to_owned()],
        ..Drawn::default()
    };
    let snapshot = Snapshot::read(&tree, &drawn);
    names
        .iter()
        .map(|name| {
            let seen = (0..)
                .map_while(|index| snapshot.at(index))
                .find(|seen| seen.name == *name)
                .expect("the node");
            snapshot.is_focusable(seen)
        })
        .collect()
}

/// Which element around a column's entries is the focus stop.
#[derive(Clone, Copy, Debug)]
enum Stop {
    List,
    Strip,
    /// A list that takes focus and names no active descendant.
    NamingNone,
    /// Nothing around the entries takes focus.
    None,
}

#[test]
fn an_entry_is_focusable_as_its_focus_stops_active_descendant() {
    let names = ["archive", "not an entry", "outside"];
    for (stop, archive) in [
        (Stop::List, true),
        (Stop::Strip, true),
        (Stop::NamingNone, false),
        (Stop::None, false),
    ] {
        assert_eq!(
            focusable_in(stop, &names),
            [archive, false, false],
            "{stop:?}"
        );
    }
}

#[test]
fn window_announcer_is_stripped_while_real_status_bars_remain() {
    let tree = json!({
        "role": "window",
        "children": [
            { "role": "group", "key": "footer", "children": [
                { "role": "status", "key": "clipboard-status", "name": "1 item copied",
                  "states": ["live-polite"], "bounds": [0.0, 736.0, 200.0, 24.0] },
            ] },
            { "role": "status", "name": "Copied\u{200b}",
              "states": ["live-assertive"], "bounds": [0.0, 0.0, 1.0, 1.0] },
        ],
    });
    let snapshot = Snapshot::read(&tree, &Drawn::default());
    let statuses: Vec<_> = (0..)
        .map_while(|index| snapshot.at(index))
        .filter(|seen| seen.role() == "status bar")
        .map(|seen| seen.name.as_str())
        .collect();
    assert_eq!(statuses, ["1 item copied"]);
}

fn nested_submenu_tree() -> serde_json::Value {
    json!({
        "role": "window",
        "children": [
            { "role": "group", "key": "kit-root", "children": [
                { "role": "menu", "key": "menu-menu", "name": "todo.txt", "states": ["modal"], "children": [
                    { "role": "menu-item", "key": "menu-item1", "name": "Offline availability", "bounds": [10.0, 20.0, 160.0, 28.0] },
                    { "role": "menu-item", "key": "menu-item2", "name": "Properties", "bounds": [10.0, 48.0, 160.0, 28.0] },
                ] },
                { "role": "menu", "key": "menu-item1-sub-menu", "name": "Offline availability", "children": [
                    { "role": "menu-item", "key": "menu-item1-sub-item0", "name": "Keep test pin", "bounds": [170.0, 20.0, 160.0, 28.0] },
                    { "role": "menu-item", "key": "menu-item1-sub-item1", "name": "Details", "bounds": [170.0, 48.0, 160.0, 28.0] },
                    { "role": "menu-item", "key": "menu-item1-sub-item2", "name": "Release pin", "bounds": [170.0, 76.0, 160.0, 28.0] },
                ] },
                { "role": "menu", "key": "menu-item1-sub-item1-sub-menu", "name": "Details", "children": [
                    { "role": "menu-item", "key": "menu-item1-sub-item1-sub-item0", "name": "Inspect nested state", "bounds": [330.0, 48.0, 160.0, 28.0] },
                ] },
            ] },
        ],
    })
}

#[test]
fn open_submenu_surfaces_nest_under_their_owning_menu() {
    let tree = nested_submenu_tree();
    let snapshot = Snapshot::read(&tree, &Drawn::default());

    let kit_root = snapshot.at(1).expect("kit-root");
    let root_menus: Vec<_> = kit_root
        .children
        .iter()
        .filter_map(|&c| snapshot.at(c))
        .filter(|s| s.role() == "menu")
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(root_menus, ["todo.txt"]);

    let root_menu_idx = kit_root
        .children
        .iter()
        .copied()
        .find(|&c| snapshot.at(c).is_some_and(|s| s.role() == "menu"))
        .expect("root menu");
    let items: Vec<_> = snapshot
        .below(root_menu_idx)
        .into_iter()
        .filter_map(|i| snapshot.at(i))
        .filter(|s| s.role() == "menu item")
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        items,
        [
            "Offline availability",
            "Keep test pin",
            "Details",
            "Inspect nested state",
            "Release pin",
            "Properties"
        ]
    );
}

#[test]
fn archive_preview_rows_keep_native_accesskit_names() {
    let tree = json!({
        "role": "window",
        "children": [
            { "role": "region", "key": "preview-pane", "name": "Preview", "children": [
                { "role": "list-box", "key": "preview-archive", "states": ["focused"],
                  "activeDescendant": "row-42", "children": [
                    { "role": "list-box-option", "key": "row-42", "name": "top.txt",
                      "description": "3 B", "states": ["selected"],
                      "bounds": [800.0, 80.0, 360.0, 28.0] },
                ] },
            ] },
        ],
    });
    let snapshot = Snapshot::read(&tree, &Drawn::default());
    let row = (0..)
        .map_while(|i| snapshot.at(i))
        .find(|s| s.key.as_deref() == Some("row-42"))
        .expect("archive member row");
    assert_eq!(row.mark, None, "archive member is not a browser entry");
    assert_eq!(row.name, "top.txt");
    assert_eq!(row.description, "3 B");
    assert!(row.children.is_empty());
}
