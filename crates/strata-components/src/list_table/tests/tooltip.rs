// SPDX-License-Identifier: MIT

use std::cell::Cell as StdCell;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, Env, Runtime, col, el};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::element::{Collection, CollectionState, RowMarks, Slot};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource};

const W: f32 = 640.0;
const H: f32 = 300.0;
const LONG_NAME: &str = "very-long-list-filename-that-must-be-middle-truncated-in-name-column.txt";

struct Host {
    listing: ListSource,
    coll: CollectionState,
    table: ListTableState,
    layout: ColumnLayout,
    widths: ColumnWidths,
    look: Look,
    marks: RowMarks,
    renaming: Option<EntryId>,
    date_version: u64,
    date_text: String,
    truncation_probes: Rc<StdCell<usize>>,
}

fn columns() -> Vec<Column<Host>> {
    vec![
        Column::primary("name", "Name").width(180.0).min(140.0),
        Column::new("modified", "Modified", |s: &Host, _| {
            Cell::text(s.date_text.clone())
        })
        .width(150.0)
        .min(110.0),
    ]
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    let probes = s.truncation_probes.clone();
    let table = ListTable::new(
        "list",
        columns(),
        |s: &Host| ListParts {
            source: &s.listing,
            collection: &s.coll,
            table: &s.table,
            layout: &s.layout,
            widths: &s.widths,
            sort: None,
            look: &s.look,
        },
        |s: &mut Host| &mut s.table,
    )
    .cells_version(|s: &Host, _| s.date_version)
    .tooltip(move |s: &Host, rcx, truncated| {
        if !s.layout.is_visible(&ColumnId::new("modified")) {
            return Some(format!("{}\n{}", rcx.entry.name, s.date_text));
        }
        probes.set(probes.get() + 1);
        truncated().then(|| rcx.entry.name.to_owned())
    });
    let base = Collection::new(
        "list",
        |s: &Host| (&s.listing, &s.coll),
        |s: &mut Host| (&s.listing, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
    .marks(|s: &Host, _| s.marks)
    .rename(
        |s: &Host| s.renaming,
        Slot::pure(|_, _| el().key("rename-editor").w(120.0).h(20.0)),
    );
    let mut layer = Vec::new();
    col().size(W, H).child(table.build(s, base, &mut layer))
}

fn runtime(hide_modified: bool) -> Runtime<Host> {
    let mut layout = ColumnLayout::new([ColumnId::new("name"), ColumnId::new("modified")]);
    if hide_modified {
        layout = layout.with_visible(&ColumnId::new("modified"), false);
    }
    let widths = ColumnWidths::new().with(&ColumnId::new("name"), 180.0);
    let mut coll = CollectionState::new();
    coll.set_groups(groups_from_runs(2, |_| (1, Arc::from("Files"))));
    let host = Host {
        listing: ListSource::new(vec![
            ListEntry::new(EntryId::new(1), "short.txt", EntryKind::File, ""),
            ListEntry::new(EntryId::new(2), LONG_NAME, EntryKind::File, ""),
        ]),
        coll,
        table: ListTableState::new(),
        layout,
        widths,
        look: Look::fallback(false),
        marks: RowMarks::default(),
        renaming: None,
        date_version: 1,
        date_text: "2026-11-12 09:00".into(),
        truncation_probes: Rc::default(),
    };
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn hover_element(
    rt: &mut Runtime<Host>,
    pred: impl Fn(&rune::ElementInfo) -> bool,
) -> Option<String> {
    let rect = rt
        .elements()
        .into_iter()
        .find(pred)
        .expect("mounted target")
        .rect;
    rt.pointer_move(rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
    rt.frame(1.0 / 60.0);
    rt.frame(rt.help_delay + 0.1);
    rt.help_tooltip().map(|(text, _)| text.to_owned())
}

fn hover_name_cell(rt: &mut Runtime<Host>, row_key: &str) -> Option<String> {
    hover_element(rt, |e| {
        e.key.as_deref() == Some("cell-name") && e.path.contains(row_key)
    })
}

#[test]
fn list_tooltip_respects_modified_column_visibility_and_skips_headers_rename_and_drag() {
    let mut rt = runtime(false);
    // Modified column visible: short name has no tooltip, truncated name shows full name only.
    assert_eq!(hover_name_cell(&mut rt, "row-1"), None);
    assert_eq!(
        hover_name_cell(&mut rt, "row-2").as_deref(),
        Some(LONG_NAME)
    );
    // Group headers never show entry tooltips.
    assert_eq!(
        hover_element(&mut rt, |e| e.key.as_deref() == Some("head-1")),
        None
    );

    // Hiding Modified column switches rows to "{name}\n{date}" without calling truncation probes.
    rt.state.truncation_probes.set(0);
    rt.state.layout = rt
        .state
        .layout
        .with_visible(&ColumnId::new("modified"), false);
    rt.frame(1.0 / 60.0);
    assert_eq!(
        rt.state.truncation_probes.get(),
        0,
        "hidden Modified column formats date without measuring truncation"
    );
    assert_eq!(
        hover_name_cell(&mut rt, "row-1").as_deref(),
        Some("short.txt\n2026-11-12 09:00")
    );

    // Live date format update via cells_version updates the tooltip.
    rt.state.date_text = "Nov 12, 2026".into();
    rt.state.date_version = 2;
    rt.frame(1.0 / 60.0);
    assert_eq!(
        hover_name_cell(&mut rt, "row-1").as_deref(),
        Some("short.txt\nNov 12, 2026")
    );

    // Suppressed while dragging or renaming.
    rt.state.marks.dragging = true;
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_name_cell(&mut rt, "row-1"), None);

    rt.state.marks.dragging = false;
    rt.state.renaming = Some(EntryId::new(1));
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_name_cell(&mut rt, "row-1"), None);
}
