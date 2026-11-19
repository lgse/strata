// SPDX-License-Identifier: MIT

use super::{
    super::{Mark, States, scan},
    fake::{Fake, Node, WINDOW, drawn},
};

fn pane(fake: &mut Fake, name: &'static str, view: &'static str, x: f64) -> (usize, usize) {
    let pane = fake.add(
        WINDOW,
        Node {
            mark: Some(Mark::Pane(view)),
            ..drawn("grouping", name, [x, 0.0, 200.0, 600.0])
        },
    );
    let list = fake.add(
        pane,
        Node {
            mark: Some(Mark::EntryContainer),
            ..drawn("list", name, [x, 40.0, 200.0, 560.0])
        },
    );
    (pane, list)
}

fn entry(fake: &mut Fake, list: usize, name: &'static str, at: [f64; 2], selected: bool) -> usize {
    fake.add(
        list,
        Node {
            mark: Some(Mark::Entry),
            states: States {
                selected,
                ..States::default()
            },
            ..drawn("list item", name, [at[0], at[1], 90.0, 30.0])
        },
    )
}

#[test]
fn panes_read_left_to_right_and_entries_top_to_bottom_then_left_to_right() {
    let mut fake = Fake::new();
    // Added right to left and bottom to top, so only the boxes can give the order.
    let (_, documents) = pane(&mut fake, "documents", "Columns", 200.0);
    let (_, home) = pane(&mut fake, "home", "Columns", 0.0);
    entry(&mut fake, documents, "b.txt", [200.0, 70.0], false);
    entry(&mut fake, documents, "a.txt", [200.0, 40.0], true);
    entry(&mut fake, home, "right", [100.0, 40.0], false);
    entry(&mut fake, home, "below", [0.0, 70.0], false);
    entry(&mut fake, home, "left", [0.0, 40.0], true);

    let scene = scan(&fake, &WINDOW);
    let observation = &scene.observation;
    let names: Vec<&str> = observation
        .panes
        .iter()
        .map(|pane| pane.name.as_str())
        .collect();
    assert_eq!(names, ["home", "documents"]);
    assert_eq!(
        observation.entry_names(Some("home")),
        ["left", "right", "below"]
    );
    assert_eq!(observation.entry_names(None), ["a.txt", "b.txt"]);
    assert_eq!(observation.all_selected(), ["left", "a.txt"]);
    assert_eq!(observation.view_mode(), Some("Columns"));
    assert_eq!(observation.window_width, Some(800));

    let row = scene.entry("below", Some("home")).copied();
    assert_eq!(
        row.map(|row| fake_name(&fake, row)),
        Some("below".to_owned())
    );
    assert_eq!(scene.pane(None).map(|pane| pane.entries.len()), Some(2));
}

fn fake_name(fake: &Fake, node: usize) -> String {
    use super::super::Backend as _;
    fake.name(&node)
}

#[test]
fn hidden_and_undrawn_nodes_are_left_out_of_the_snapshot() {
    let mut fake = Fake::new();
    let (_, list) = pane(&mut fake, "home", "Icons", 0.0);
    entry(&mut fake, list, "shown", [0.0, 40.0], false);
    fake.add(
        list,
        Node {
            mark: Some(Mark::Entry),
            role: "list item",
            name: "scrolled away",
            ..Node::default()
        },
    );
    let hidden = fake.add(
        WINDOW,
        Node {
            hidden: true,
            ..drawn("panel", "", [0.0, 0.0, 800.0, 600.0])
        },
    );
    fake.add(
        hidden,
        drawn("dialog", "Rename", [100.0, 100.0, 300.0, 200.0]),
    );
    fake.add(
        WINDOW,
        Node {
            mark: Some(Mark::Pane("Icons")),
            role: "grouping",
            name: "not drawn",
            ..Node::default()
        },
    );

    let observation = scan(&fake, &WINDOW).observation;
    assert_eq!(observation.entry_names(None), ["shown"]);
    assert_eq!(observation.panes.len(), 1);
    assert_eq!(
        observation.dialog, None,
        "a dialog under a hidden node is not open"
    );
}

#[test]
fn focus_names_the_pane_that_holds_it_and_the_entry_it_is_on() {
    let mut fake = Fake::new();
    let (_, home) = pane(&mut fake, "home", "Columns", 0.0);
    let (documents_pane, documents) = pane(&mut fake, "documents", "Columns", 200.0);
    entry(&mut fake, home, "documents", [0.0, 40.0], true);
    let report = entry(&mut fake, documents, "report.txt", [200.0, 40.0], false);

    assert_eq!(scan(&fake, &WINDOW).observation.focused_pane_name(), None);

    fake.focus = Some(report);
    let observation = scan(&fake, &WINDOW).observation;
    assert_eq!(observation.focused_entry.as_deref(), Some("report.txt"));
    assert_eq!(observation.focused_pane_name(), Some("documents"));
    assert_eq!(observation.directory(), Some("documents"));
    let state = observation.app_state();
    assert_eq!(state.panes[1].focused.as_deref(), Some("report.txt"));

    // The pane itself takes focus while its directory is empty.
    fake.focus = Some(documents_pane);
    let observation = scan(&fake, &WINDOW).observation;
    assert_eq!(observation.focused_entry, None);
    assert_eq!(observation.focused_pane_name(), Some("documents"));
}

#[test]
fn dialog_menu_preview_peek_results_and_field_are_found_by_role_and_label() {
    let mut fake = Fake::new();
    let observation = scan(&fake, &WINDOW).observation;
    assert_eq!(
        (
            &observation.dialog,
            &observation.menu,
            &observation.preview,
            observation.peek,
            &observation.results,
            &observation.field
        ),
        (&None, &None, &None, false, &None, &None)
    );

    fake.add(
        WINDOW,
        drawn("alert", "Delete 2 items?", [0.0; 4].map(|_: f64| 10.0)),
    );
    let menu = fake.add(WINDOW, drawn("menu", "", [10.0, 10.0, 100.0, 90.0]));
    fake.add(menu, drawn("menu item", "Open", [10.0, 10.0, 100.0, 30.0]));
    fake.add(menu, drawn("separator", "", [10.0, 40.0, 100.0, 2.0]));
    fake.add(
        menu,
        drawn("menu item", "Rename…", [10.0, 42.0, 100.0, 30.0]),
    );
    let preview = fake.add(
        WINDOW,
        drawn("grouping", "Preview", [600.0, 0.0, 200.0, 600.0]),
    );
    fake.add(
        preview,
        Node {
            text: Some("12 KB"),
            ..drawn("label", "", [600.0, 40.0, 200.0, 20.0])
        },
    );
    fake.add(
        WINDOW,
        drawn("panel", "Folder peek", [300.0, 300.0, 200.0, 200.0]),
    );
    let results = fake.add(
        WINDOW,
        drawn("table", "Search results", [0.0, 40.0, 600.0, 500.0]),
    );
    fake.add(
        results,
        drawn("table cell", "todo.txt", [0.0, 40.0, 600.0, 30.0]),
    );
    fake.add(results, drawn("table cell", "", [0.0, 70.0, 600.0, 30.0]));
    let filter = fake.add(
        WINDOW,
        Node {
            field: true,
            text: Some("todo"),
            states: States {
                editable: true,
                ..States::default()
            },
            ..drawn("text", "Filter", [0.0, 0.0, 200.0, 30.0])
        },
    );
    fake.focus = Some(filter);

    let observation = scan(&fake, &WINDOW).observation;
    assert_eq!(observation.dialog.as_deref(), Some("Delete 2 items?"));
    assert_eq!(
        observation.menu,
        Some(vec!["Open".to_owned(), "Rename…".to_owned()])
    );
    assert_eq!(
        observation.preview,
        Some(vec!["Preview".to_owned(), "12 KB".to_owned()])
    );
    assert!(observation.peek);
    assert_eq!(observation.results, Some(vec!["todo.txt".to_owned()]));
    assert_eq!(observation.matches(None), ["todo.txt"]);
    let field = observation.field.expect("the focused field is reported");
    assert_eq!(
        (field.name.as_str(), field.text.as_str()),
        ("Filter", "todo")
    );
}

#[test]
fn a_search_results_list_inside_a_dialog_is_no_filters_results() {
    let mut fake = Fake::new();
    let (_, home) = pane(&mut fake, "home", "Columns", 0.0);
    entry(&mut fake, home, "notes", [0.0, 40.0], true);
    let palette = fake.add(
        WINDOW,
        drawn(
            "dialog",
            "Search files and folders…",
            [200.0, 100.0, 400.0, 300.0],
        ),
    );
    let results = fake.add(
        palette,
        drawn("list", "Search results", [200.0, 160.0, 400.0, 200.0]),
    );
    fake.add(
        results,
        drawn(
            "list item",
            "/home/u/notes.txt",
            [200.0, 160.0, 400.0, 30.0],
        ),
    );

    let observation = scan(&fake, &WINDOW).observation;
    assert_eq!(
        observation.results, None,
        "a palette's list is not the filter's"
    );
    assert_eq!(observation.matches(None), ["notes"]);
}

/// The open dialog is the one on top: on a toolkit that draws a message inside a dialog, the
/// innermost; elsewhere the first, which a dialog beside it does not displace.
#[test]
fn the_open_dialog_is_the_innermost_where_the_toolkit_nests_them() {
    let opened = |nests: bool| {
        let mut fake = Fake::new();
        fake.nests_dialogs = nests;
        let settings = fake.add(
            WINDOW,
            drawn("dialog", "Settings", [0.0, 0.0, 800.0, 600.0]),
        );
        let body = fake.add(settings, drawn("panel", "", [0.0, 0.0, 800.0, 600.0]));
        fake.add(
            body,
            drawn(
                "dialog",
                "Reset all keybinds?",
                [200.0, 200.0, 400.0, 200.0],
            ),
        );
        fake.add(WINDOW, drawn("dialog", "Beside", [0.0, 0.0, 10.0, 10.0]));
        scan(&fake, &WINDOW).observation.dialog
    };
    assert_eq!(opened(true).as_deref(), Some("Reset all keybinds?"));
    assert_eq!(opened(false).as_deref(), Some("Settings"));
}

/// Among messages drawn side by side inside a dialog, the one on top is the last drawn; a live
/// error line, an `alert` inside a dialog, is no dialog of its own and leaves it as it is.
#[test]
fn the_dialog_on_top_is_the_last_drawn_and_an_error_line_is_none() {
    let opened = |nests: bool| {
        let mut fake = Fake::new();
        fake.nests_dialogs = nests;
        let settings = fake.add(
            WINDOW,
            drawn("dialog", "Settings", [0.0, 0.0, 800.0, 600.0]),
        );
        fake.add(
            settings,
            drawn("alert", "Ctrl+C is taken", [10.0, 10.0, 200.0, 20.0]),
        );
        fake.add(
            settings,
            drawn("dialog", "Under", [100.0, 100.0, 400.0, 200.0]),
        );
        let over = fake.add(
            settings,
            drawn("dialog", "Over", [120.0, 120.0, 400.0, 200.0]),
        );
        fake.add(
            over,
            drawn("alert", "Name is empty", [130.0, 130.0, 200.0, 20.0]),
        );
        scan(&fake, &WINDOW).observation.dialog
    };
    assert_eq!(opened(true).as_deref(), Some("Over"));
    assert_eq!(opened(false).as_deref(), Some("Settings"));
}
