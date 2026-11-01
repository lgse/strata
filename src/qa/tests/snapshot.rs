// SPDX-License-Identifier: MIT

use serde_json::json;

use super::{
    super::{
        protocol::FieldCheck,
        state::{self, EntryObservation, FieldState, Observation, PaneObservation},
    },
    predicate,
};

fn entry(name: &str, selected: bool) -> EntryObservation {
    EntryObservation {
        name: name.to_owned(),
        selected,
        focused: false,
    }
}

fn pane(name: &str, entries: Vec<EntryObservation>) -> PaneObservation {
    PaneObservation {
        name: name.to_owned(),
        view: "Columns",
        entries,
    }
}

#[test]
fn the_current_directory_is_the_focused_pane_or_else_the_deepest() {
    let mut observation = Observation {
        panes: vec![pane("root", vec![]), pane("documents", vec![])],
        ..Observation::default()
    };
    assert_eq!(observation.directory(), Some("documents"));
    observation.focused_pane = Some(0);
    assert_eq!(observation.directory(), Some("root"));
    assert!(
        observation
            .evaluate(&predicate(json!({ "directory": "root" })))
            .is_ok()
    );
}

#[test]
fn entry_predicates_read_the_named_pane_or_the_deepest() {
    let observation = Observation {
        panes: vec![
            pane(
                "root",
                vec![entry("archive", true), entry("todo.txt", false)],
            ),
            pane("archive", vec![entry("old.txt", false)]),
        ],
        focused_entry: Some("archive".to_owned()),
        focused_pane: Some(0),
        window_width: Some(1200),
        ..Observation::default()
    };
    for holding in [
        json!({ "entries": ["old.txt"] }),
        json!({ "entries": ["archive", "todo.txt"], "dir": "root" }),
        json!({ "entries": { "contains": ["todo.txt"] }, "dir": "root" }),
        json!({ "entries": { "excludes": ["todo.txt"] } }),
        json!({ "selected": ["archive"], "dir": "root" }),
        json!({ "selected": [] }),
        json!({ "panes": ["root", "archive"] }),
        json!({ "focused": "archive" }),
        json!({ "view_mode": "Columns" }),
    ] {
        assert!(
            observation.evaluate(&predicate(holding.clone())).is_ok(),
            "{holding} should hold"
        );
    }
    assert!(observation.entry_selected("archive", None));
    assert!(!observation.entry_selected("archive", Some("archive")));
    assert!(!observation.entry_selected("todo.txt", None));
    for failing in [
        json!({ "entries": ["todo.txt", "archive"], "dir": "root" }),
        json!({ "entries": { "contains": ["todo.txt"] } }),
        json!({ "focused": null }),
        json!({ "directory": "archive" }),
    ] {
        assert!(
            observation.evaluate(&predicate(failing.clone())).is_err(),
            "{failing} should not hold"
        );
    }
    let failure = observation
        .evaluate(&predicate(
            json!({ "selected": ["todo.txt"], "dir": "root" }),
        ))
        .expect_err("the selection differs");
    assert_eq!(failure.actual, json!(["archive"]));
    assert_eq!(
        failure.predicate,
        json!({ "selected": ["todo.txt"], "dir": "root" })
    );
    assert_eq!(
        observation
            .evaluate(&predicate(json!({ "entries": ["x"], "dir": "missing" })))
            .expect_err("no such pane")
            .actual,
        json!([])
    );
}

#[test]
fn the_snapshot_reports_selection_and_focus_per_pane() {
    let mut focused = entry("todo.txt", true);
    focused.focused = true;
    let observation = Observation {
        panes: vec![pane("root", vec![entry("archive", false), focused])],
        focused_entry: Some("todo.txt".to_owned()),
        focused_pane: Some(0),
        dialog: Some("Unable to rename item".to_owned()),
        field: Some(FieldState {
            name: "Rename".to_owned(),
            text: "todo.txt".to_owned(),
            caret: 8,
            selection: Some([0, 4]),
            focused: true,
            showing: true,
        }),
        ..Observation::default()
    };
    assert_eq!(
        serde_json::to_value(observation.app_state()).expect("the snapshot should encode"),
        json!({
            "view_mode": "Columns",
            "directory": "root",
            "panes": [{ "name": "root", "selected": ["todo.txt"], "focused": "todo.txt" }],
            "focused": "todo.txt",
            "focused_pane": "root",
            "dialog": "Unable to rename item",
            "menu": null,
            "preview": null,
            "field": {
                "name": "Rename", "text": "todo.txt", "caret": 8,
                "selection": [0, 4], "focused": true,
            },
            "matches": ["archive", "todo.txt"],
        })
    );
}

#[test]
fn entry_pane_and_width_predicates_read_the_snapshot() {
    let observation = Observation {
        panes: vec![
            pane(
                "root",
                vec![entry("archive", true), entry("todo.txt", false)],
            ),
            pane("archive", vec![entry("old.txt", false)]),
        ],
        focused_pane: Some(0),
        window_width: Some(420),
        ..Observation::default()
    };
    for holding in [
        json!({ "pane": "archive" }),
        json!({ "entry": "todo.txt" }),
        json!({ "entry": "old.txt", "dir": "archive" }),
        json!({ "entry_gone": "todo.txt", "dir": "archive" }),
        json!({ "entry_gone": "missing.txt" }),
        json!({ "window_width": 420 }),
    ] {
        assert!(
            observation.evaluate(&predicate(holding.clone())).is_ok(),
            "{holding} should hold"
        );
    }
    for failing in [
        json!({ "pane": "root" }),
        json!({ "entry": "todo.txt", "dir": "archive" }),
        json!({ "entry_gone": "todo.txt" }),
        json!({ "window_width": 640 }),
    ] {
        assert!(
            observation.evaluate(&predicate(failing.clone())).is_err(),
            "{failing} should not hold"
        );
    }
}

#[test]
fn dialog_menu_preview_and_match_predicates_read_the_snapshot() {
    let observation = Observation {
        panes: vec![pane("root", vec![entry("readme.md", false)])],
        focused_pane: Some(0),
        dialog: Some("Unable to rename item".to_owned()),
        menu: Some(vec!["New File".to_owned(), "New Folder".to_owned()]),
        preview: Some(vec!["Preview".to_owned(), "beta source\n".to_owned()]),
        results: Some(vec![
            "match-note.txt".to_owned(),
            "match-note.txt".to_owned(),
        ]),
        ..Observation::default()
    };
    for holding in [
        json!({ "dialog": "Unable to rename item" }),
        json!({ "dialog": true }),
        json!({ "focused_pane": "root" }),
        json!({ "focused": null }),
        json!({ "menu_items": {} }),
        json!({ "menu_items": { "contains": ["New Folder"], "excludes": ["Quick preview"] } }),
        json!({ "preview": "beta source" }),
        json!({ "preview": true }),
        json!({ "peek": false }),
        json!({ "matches": { "count": 2 } }),
        json!({ "matches": { "at_least": 2, "ends_with": ["match-note.txt"] } }),
        json!({ "matches": ["match-note.txt", "match-note.txt"] }),
        json!({ "any": [{ "focused": true }, { "focused_pane": true }] }),
    ] {
        assert!(
            observation.evaluate(&predicate(holding.clone())).is_ok(),
            "{holding} should hold"
        );
    }
    for failing in [
        json!({ "dialog": null }),
        json!({ "menu_items": null }),
        json!({ "preview": null }),
        json!({ "preview": "alpha source" }),
        json!({ "peek": true }),
        json!({ "matches": { "count": 3 } }),
        json!({ "matches": { "at_least": 3 } }),
        json!({ "any": [{ "focused": true }, { "dialog": null }] }),
    ] {
        assert!(
            observation.evaluate(&predicate(failing.clone())).is_err(),
            "{failing} should not hold"
        );
    }
    let closed = Observation {
        panes: vec![pane("root", vec![entry("readme.md", false)])],
        ..Observation::default()
    };
    for holding in [
        json!({ "dialog": null }),
        json!({ "menu_items": null }),
        json!({ "preview": null }),
        json!({ "matches": ["readme.md"] }),
    ] {
        assert!(
            closed.evaluate(&predicate(holding.clone())).is_ok(),
            "{holding} should hold with nothing open"
        );
    }
    assert!(
        closed
            .evaluate(&predicate(json!({ "preview": true })))
            .is_err()
    );
    assert!(
        closed
            .evaluate(&predicate(json!({ "menu_items": {} })))
            .is_err()
    );
}

#[test]
fn selection_focus_and_negated_predicates_read_every_pane() {
    let mut cursor = entry("notes.txt", false);
    cursor.focused = true;
    let observation = Observation {
        panes: vec![
            pane(
                "root",
                vec![entry("documents", true), entry("todo.txt", true)],
            ),
            pane("documents", vec![cursor, entry("report.md", true)]),
        ],
        focused_entry: Some("notes.txt".to_owned()),
        focused_pane: Some(1),
        results: Some(vec!["notes.txt".to_owned()]),
        ..Observation::default()
    };
    for holding in [
        json!({ "all_selected": ["documents", "todo.txt", "report.md"] }),
        json!({ "panes": { "ends_with": ["root", "documents"], "excludes": ["archive"] } }),
        json!({ "focused": { "one_of": ["readme.md", "notes.txt"] } }),
        json!({ "focused_in_matches": true }),
        json!({ "not": { "selected": [], "dir": "root" } }),
        json!({ "not": { "directory": "root" } }),
    ] {
        assert!(
            observation.evaluate(&predicate(holding.clone())).is_ok(),
            "{holding} should hold"
        );
    }
    for failing in [
        json!({ "all_selected": [] }),
        json!({ "panes": { "ends_with": ["documents", "root"] } }),
        json!({ "focused": { "one_of": ["readme.md"] } }),
        json!({ "focused_in_matches": false }),
        json!({ "not": { "selected": ["report.md"] } }),
    ] {
        assert!(
            observation.evaluate(&predicate(failing.clone())).is_err(),
            "{failing} should not hold"
        );
    }
}

#[test]
fn window_geometry_checks_use_whole_pixel_arithmetic() {
    let window = [0, 0, 1200, 760];
    assert!(state::on_screen([10, 700, 100, 60], window));
    assert!(!state::on_screen([10, 701, 100, 60], window));
    assert!(!state::on_screen([10, 10, 100, 0], window));
    assert!(state::contains([10, 20, 30, 40], (10, 59)));
    assert!(!state::contains([10, 20, 30, 40], (40, 30)));

    let rows: Vec<(String, [i64; 4])> = (0..8)
        .map(|index| (format!("folder-{index:03}"), [0, index * 30, 100, 30]))
        .collect();
    assert_eq!(
        state::middle_visible(&rows, [0, 40, 100, 120], "folder-000"),
        Some("folder-003".to_owned())
    );
    assert_eq!(
        state::middle_visible(&rows, [0, -10, 100, 120], "folder-000"),
        None,
        "nothing scrolled yet while the first visible row is the first entry"
    );
    assert_eq!(
        state::middle_visible(&rows, [0, 400, 100, 60], "folder-000"),
        None
    );
}

#[test]
fn a_field_check_matches_only_the_properties_it_names() {
    let field = FieldState {
        name: "Rename".to_owned(),
        text: "report.txt".to_owned(),
        caret: 6,
        selection: None,
        focused: true,
        showing: true,
    };
    let check = |json: serde_json::Value| -> FieldCheck {
        serde_json::from_value(json).expect("the field check should parse")
    };
    for holding in [
        json!({}),
        json!({ "name": "Rename", "text": "report.txt", "focused": true }),
        json!({ "text": { "ends_with": ".txt", "starts_with": "rep" } }),
        json!({ "caret": 6, "selection": null }),
        json!({ "caret": { "min": 1, "max": 9 } }),
    ] {
        assert!(
            state::field_matches(&check(holding.clone()), Some(&field)),
            "{holding} should match"
        );
    }
    for failing in [
        json!({ "text": "report" }),
        json!({ "caret": { "min": 7, "max": 9 } }),
        json!({ "selection": [0, 6] }),
        json!({ "focused": false }),
    ] {
        assert!(
            !state::field_matches(&check(failing.clone()), Some(&field)),
            "{failing} should not match"
        );
    }
    assert!(!state::field_matches(&check(json!({})), None));
}

#[test]
fn an_editor_is_within_the_viewport_only_inside_the_pane_and_window() {
    assert!(state::within_viewport([210, 40, 200, 24], 200, 420));
    assert!(state::within_viewport([200, 40, 220, 24], 200, 420));
    assert!(!state::within_viewport([190, 40, 100, 24], 200, 420));
    assert!(!state::within_viewport([210, 40, 221, 24], 200, 420));
    assert!(!state::within_viewport([210, 40, 100, 0], 200, 420));
    assert!(!state::within_viewport([420, 40, 0, 24], 200, 420));
}

#[test]
fn a_created_editor_starts_beside_the_pane_and_ends_inside_the_window() {
    assert!(state::within_window([210, 40, 200, 24], 200, 420));
    assert!(state::within_window([0, 40, 2, 24], -30, 420));
    assert!(!state::within_window([-1, 40, 100, 24], -30, 420));
    assert!(!state::within_window([190, 40, 100, 24], 200, 420));
    assert!(!state::within_window([210, 40, 211, 24], 200, 420));
    assert!(!state::within_window([210, 40, 1, 24], 200, 420));
}

#[test]
fn a_box_is_inside_another_on_the_axes_asked_for() {
    use super::super::protocol::PointAxis;
    let viewport = [200, 100, 400, 300];
    assert!(state::inside([210, 110, 100, 20], viewport, None));
    assert!(!state::inside([210, 390, 100, 20], viewport, None));
    assert!(!state::inside([150, 110, 100, 20], viewport, None));
    assert!(state::inside(
        [150, 110, 100, 20],
        viewport,
        Some(PointAxis::Y)
    ));
    assert!(!state::inside(
        [210, 110, 0, 20],
        viewport,
        Some(PointAxis::Y)
    ));
    assert_eq!(
        state::collapse_whitespace(" doesn't\n support  Trash "),
        "doesn't support Trash"
    );
}
