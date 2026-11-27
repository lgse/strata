// SPDX-License-Identifier: MIT

use std::borrow::Cow;

use rune::{Cx, El, Env, Runtime, Tx, col, el};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::element::{RowMarks, Slot};
use crate::collection::selection::Listing;
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use crate::scroll::Autoscroll;

const W: f32 = 340.0;
const H: f32 = 300.0;
const LONG_NAME: &str = "extremely-long-column-filename-that-must-truncate-in-standard-width.txt";

struct Sources {
    list: ListSource,
    date: Option<String>,
    version: u64,
}

impl ColumnSources for Sources {
    fn len(&self) -> usize {
        1
    }

    fn source(&self, _depth: usize) -> &dyn EntrySource {
        &self.list
    }

    fn version(&self, _depth: usize) -> u64 {
        1
    }

    fn title(&self, _depth: usize) -> &str {
        "Folder"
    }

    fn open_child(&self, _depth: usize) -> Option<EntryId> {
        None
    }

    fn tooltip_version(&self, _depth: usize, _index: usize) -> u64 {
        self.version
    }

    fn tooltip(&self, _depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let date = self.date.as_deref()?;
        let name = self.list.entry(index).name;
        Some(Cow::Owned(format!("{name}\n{date}")))
    }
}

struct Host {
    sources: Sources,
    columns: ColumnsState,
    look: Look,
    marks: RowMarks,
    renaming: Option<EntryId>,
}

fn view_of() -> ColumnsView<Host> {
    ColumnsView::new(
        |s: &Host| ColumnsParts {
            sources: &s.sources,
            state: &s.columns,
            look: &s.look,
        },
        |s: &mut Host| ColumnsPartsMut {
            sources: &s.sources,
            state: &mut s.columns,
            look: &s.look,
        },
    )
    .marks(|s: &Host, _, _| s.marks)
    .collection(|_, _, c| {
        c.rename(
            |s: &Host| s.renaming,
            Slot::pure(|_, _| el().key("rename-editor").w(120.0).h(20.0)),
        )
    })
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(W, H).child(view_of().build(s))
}

fn runtime(date: Option<&str>) -> Runtime<Host> {
    let look = Look::fallback(false);
    let list = ListSource::new(vec![
        ListEntry::new(EntryId::new(1), "short.txt", EntryKind::File, ""),
        ListEntry::new(EntryId::new(2), LONG_NAME, EntryKind::File, ""),
    ]);
    let mut columns = ColumnsState::new("t", Autoscroll::new());
    let mut tx = Tx::default();
    columns.push(
        Opening::new(&list, Listing::Complete),
        Focus::No,
        &look,
        &mut tx,
    );
    let host = Host {
        sources: Sources {
            list,
            date: date.map(str::to_owned),
            version: 1,
        },
        columns,
        look,
        marks: RowMarks::default(),
        renaming: None,
    };
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt
}

fn hover_row(rt: &mut Runtime<Host>, row_key: &str) -> Option<String> {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(row_key))
        .expect("mounted row")
        .rect;
    rt.pointer_move(rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
    rt.frame(1.0 / 60.0);
    rt.frame(rt.help_delay + 0.1);
    rt.help_tooltip().map(|(text, _)| text.to_owned())
}

#[test]
fn columns_row_shows_host_tooltip_updates_live_and_suppresses_during_rename_drag_or_peek() {
    let mut rt = runtime(Some("2026-11-12 09:00"));
    assert_eq!(
        hover_row(&mut rt, "row-1").as_deref(),
        Some("short.txt\n2026-11-12 09:00")
    );

    // Updating tooltip_version refreshes the tooltip live.
    rt.state.sources.date = Some("Nov 12, 2026".into());
    rt.state.sources.version = 2;
    rt.frame(1.0 / 60.0);
    assert_eq!(
        hover_row(&mut rt, "row-1").as_deref(),
        Some("short.txt\nNov 12, 2026")
    );

    // Suppressed while dragging, peek-anchored, or renaming.
    rt.state.marks.dragging = true;
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_row(&mut rt, "row-1"), None);

    rt.state.marks.dragging = false;
    rt.state.marks.peek_anchor = true;
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_row(&mut rt, "row-1"), None);

    rt.state.marks.peek_anchor = false;
    rt.state.renaming = Some(EntryId::new(1));
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_row(&mut rt, "row-1"), None);
}

#[test]
fn columns_row_without_date_shows_tooltip_only_when_name_is_truncated() {
    let mut rt = runtime(None);
    assert_eq!(hover_row(&mut rt, "row-1"), None);
    assert_eq!(hover_row(&mut rt, "row-2").as_deref(), Some(LONG_NAME));
}
