// SPDX-License-Identifier: MIT

use rune::{Cx, El, Env, Runtime, Tx, col};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::selection::Listing;
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use crate::scroll::Autoscroll;

struct Sources(Vec<ListSource>);

impl ColumnSources for Sources {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn source(&self, depth: usize) -> &dyn EntrySource {
        &self.0[depth]
    }
    fn version(&self, _depth: usize) -> u64 {
        1
    }
    fn title(&self, _depth: usize) -> &str {
        "Folder"
    }
    fn open_child(&self, depth: usize) -> Option<EntryId> {
        (depth + 1 < self.0.len()).then(|| self.0[depth].id_at(0))
    }
}

struct Host {
    sources: Sources,
    columns: ColumnsState,
    look: Look,
    trailing_edge: bool,
    active_header: bool,
}

fn listing(depth: u64) -> ListSource {
    ListSource::new(vec![ListEntry::new(
        EntryId::new(depth * 10 + 1),
        "item",
        EntryKind::Folder,
        "",
    )])
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
    .trailing_edge(|s: &Host| s.trailing_edge)
    .active_header(|s: &Host| s.active_header)
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(900.0, 400.0).child(view_of().build(s))
}

fn host() -> Runtime<Host> {
    let look = Look::fallback(false);
    let mut state = Host {
        sources: Sources(vec![listing(0), listing(1)]),
        columns: ColumnsState::new("t", Autoscroll::new()),
        look,
        trailing_edge: true,
        active_header: true,
    };
    let mut tx = Tx::default();
    for depth in 0..2 {
        let source = &state.sources.0[depth];
        state.columns.push(
            Opening::new(source, Listing::Complete),
            Focus::Column,
            &state.look,
            &mut tx,
        );
    }
    let mut rt = Runtime::new(state, view, (900.0, 400.0), Env::new(false, 1.0));
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn has_key(rt: &Runtime<Host>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|el| !el.exiting && el.key.as_deref() == Some(key))
}

#[test]
fn trailing_edge_omits_only_the_rightmost_column_divider_line() {
    let mut rt = host();
    let col0 = rt.state.columns.column_key(0).expect("column 0");
    let col1 = rt.state.columns.column_key(1).expect("column 1");

    assert!(has_key(&rt, &keys::edge_line(&col0)));
    assert!(has_key(&rt, &keys::edge_line(&col1)));
    assert!(has_key(&rt, &keys::edge_accent(&col1)));

    rt.update(|s, _| s.trailing_edge = false);
    rt.frame(0.0);

    assert!(
        has_key(&rt, &keys::edge_line(&col0)),
        "between-column divider stays"
    );
    assert!(
        has_key(&rt, &keys::edge_accent(&col0)),
        "between-column hover accent stays"
    );
    assert!(
        !has_key(&rt, &keys::edge_line(&col1)),
        "rightmost column omits its static line when an adjacent divider owns the seam"
    );
    assert!(
        !has_key(&rt, &keys::edge_accent(&col1)),
        "rightmost column omits duplicate hover accent beside the adjacent divider"
    );
    assert!(
        has_key(&rt, &keys::edge(&col1)),
        "rightmost column keeps its resize handle"
    );

    rt.update(|s, _| s.trailing_edge = true);
    rt.frame(0.0);
    assert!(has_key(&rt, &keys::edge_line(&col1)));
}

#[test]
fn active_header_suppresses_and_restores_the_active_column_indicator() {
    let mut rt = host();
    let col0 = rt.state.columns.column_key(0).expect("column 0");
    let col1 = rt.state.columns.column_key(1).expect("column 1");

    assert!(!has_key(&rt, &keys::indicator(&col0)));
    assert!(has_key(&rt, &keys::indicator(&col1)));

    rt.update(|s, _| s.active_header = false);
    rt.frame(0.0);
    assert!(
        !has_key(&rt, &keys::indicator(&col1)),
        "active column yields its focus bar when another pane owns focus"
    );

    rt.update(|s, _| s.active_header = true);
    rt.frame(0.0);
    assert!(
        has_key(&rt, &keys::indicator(&col1)),
        "active column restores its focus bar when focus returns"
    );
}
