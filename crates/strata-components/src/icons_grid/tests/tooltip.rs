// SPDX-License-Identifier: MIT

//! Behavioral hover-tooltip tests for [`IconsGrid::tooltip`].

use rune::{Cx, El, Env, Runtime, col, el};
use strata_kit::look::Look;

use crate::collection::element::{Collection, CollectionState, RowMarks, Slot};
use crate::contract::{EntryId, EntryKind, ListEntry, ListSource};
use crate::icons_grid::{IconsGrid, IconsGridState, IconsParts};

const W: f32 = 420.0;
const H: f32 = 300.0;
const LONG_NAME: &str = "2026-10-07-release-candidate-build-final-v3-with-a-very-long-tail.tar.gz";

struct Host {
    src: ListSource,
    coll: CollectionState,
    grid: IconsGridState,
    look: Look,
    date: Option<String>,
    version: u64,
    marks: RowMarks,
    renaming: Option<EntryId>,
}

fn base(_: &Host) -> Collection<Host> {
    Collection::new(
        "grid",
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
    .marks(|s: &Host, _| s.marks)
    .rename(
        |s: &Host| s.renaming,
        Slot::pure(|_, _| el().key("rename-editor").w(80.0).h(20.0)),
    )
}

fn grid() -> IconsGrid<Host> {
    IconsGrid::new(
        "grid",
        base,
        |s: &Host| IconsParts {
            source: &s.src,
            collection: &s.coll,
            grid: &s.grid,
            size: 64.0,
            listing: 1,
            look: &s.look,
            thumbs: None,
        },
        |s: &mut Host| &mut s.grid,
    )
    .tooltip(
        |s: &Host, _| s.version,
        |s: &Host, rcx, truncated| match s.date.as_deref().filter(|d| !d.is_empty()) {
            Some(date) => Some(format!("{}\n{date}", rcx.entry.name)),
            None if truncated => Some(rcx.entry.name.to_owned()),
            None => None,
        },
    )
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(W, H).child(grid().build(s))
}

fn runtime(date: Option<&str>) -> Runtime<Host> {
    let host = Host {
        src: ListSource::new(vec![
            ListEntry::new(EntryId::new(1), "note.txt", EntryKind::File, ""),
            ListEntry::new(EntryId::new(2), LONG_NAME, EntryKind::File, ""),
        ]),
        coll: CollectionState::new(),
        grid: IconsGridState::new(),
        look: Look::fallback(false),
        date: date.map(str::to_owned),
        version: 1,
        marks: RowMarks::default(),
        renaming: None,
    };
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn hover_tile_icon(rt: &mut Runtime<Host>, row_key: &str) -> Option<String> {
    let prefix = format!("{row_key}/icon");
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| e.path.ends_with(&prefix))
        .expect("mounted tile icon")
        .rect;
    rt.pointer_move(rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
    rt.frame(1.0 / 60.0);
    rt.frame(rt.help_delay + 0.1);
    rt.help_tooltip().map(|(text, _)| text.to_owned())
}

#[test]
fn icons_tile_tooltip_shows_name_and_date_updates_live_and_suppresses_during_rename_or_drag() {
    let mut rt = runtime(Some("2026-11-12 09:00"));
    assert_eq!(
        hover_tile_icon(&mut rt, "row-1").as_deref(),
        Some("note.txt\n2026-11-12 09:00")
    );

    rt.state.date = Some("Nov 12, 2026".into());
    rt.state.version = 2;
    rt.frame(1.0 / 60.0);
    assert_eq!(
        hover_tile_icon(&mut rt, "row-1").as_deref(),
        Some("note.txt\nNov 12, 2026")
    );

    rt.state.marks.dragging = true;
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_tile_icon(&mut rt, "row-1"), None);

    rt.state.marks.dragging = false;
    rt.state.marks.peek_anchor = true;
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_tile_icon(&mut rt, "row-1"), None);

    rt.state.marks.peek_anchor = false;
    rt.state.renaming = Some(EntryId::new(1));
    rt.frame(1.0 / 60.0);
    assert_eq!(hover_tile_icon(&mut rt, "row-1"), None);
}

#[test]
fn icons_tile_without_date_shows_tooltip_only_when_caption_is_truncated() {
    let mut rt = runtime(None);
    assert_eq!(hover_tile_icon(&mut rt, "row-1"), None);
    assert_eq!(
        hover_tile_icon(&mut rt, "row-2").as_deref(),
        Some(LONG_NAME)
    );
}
