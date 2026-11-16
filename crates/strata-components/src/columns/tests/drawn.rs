// SPDX-License-Identifier: MIT

//! Drop routing (`ColumnsView::drop_target_at` and `ColumnsState::drop_column`) while a column's
//! width springs (`El::drawn_rect_into`, U-C2-3).

use rune::{Cx, El, Env, Runtime, Spring, Tx, col};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::selection::Listing;
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource, Point};
use crate::scroll::Autoscroll;

const W: f32 = 900.0;
const H: f32 = 400.0;
const DT: f64 = 1.0 / 60.0;
const SPRING: Spring = Spring::new(0.22, 0.0);

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
}

fn listing(depth: u64) -> ListSource {
    ListSource::new(
        (0..5)
            .map(|i| {
                ListEntry::new(
                    EntryId::new(depth * 100 + i + 1),
                    format!("folder {i}"),
                    EntryKind::Folder,
                    "",
                )
            })
            .collect(),
    )
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
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    col().size(W, H).child(view_of().build(s))
}

fn host() -> Runtime<Host> {
    let look = Look::fallback(false);
    let mut host = Host {
        sources: Sources(vec![listing(0), listing(1)]),
        columns: ColumnsState::new("t", Autoscroll::new()),
        look,
    };
    let mut tx = Tx::default();
    for depth in 0..2 {
        let source = &host.sources.0[depth];
        host.columns.push(
            Opening::new(source, Listing::Complete),
            Focus::No,
            &host.look,
            &mut tx,
        );
    }
    let mut rt = Runtime::new(host, view, (W, H), Env::new(false, 1.0));
    rt.set_system_reduced_motion(false);
    rt.poll = Some(|s: &mut Host, t, tx| view_of().tick(s, t, tx));
    rt.frame(0.0);
    for _ in 0..24 {
        rt.frame(DT);
    }
    rt
}

#[test]
fn drop_targets_follow_drawn_columns_through_a_width_spring() {
    let mut rt = host();
    let v = view_of();
    let col0_first = EntryId::new(1);
    let col1_first = EntryId::new(101);
    // y = 50 sits inside row 0 below the 36 pt column header.
    let at_350 = Point::new(350.0, 50.0);
    let at_420 = Point::new(420.0, 50.0);

    // At rest (two 300 pt columns), x = 350 and x = 420 are over column 1's first folder row.
    assert_eq!(
        v.drop_target_at(&rt.state, at_350),
        Some((1, Some(col1_first)))
    );

    // Spring column 0 wider from 300 to 500: on the first animated frame, layout already places
    // column 1 at x = 500..800, while the frame still draws column 1 covering x = 350.
    rt.update(|s, tx| {
        s.columns.set_column_width(0, 500.0);
        tx.animated();
        tx.motion(SPRING);
    });
    rt.frame(DT);
    assert_eq!(
        v.drop_target_at(&rt.state, at_350),
        Some((1, Some(col1_first))),
        "first widening frame keeps x = 350 over column 1's drawn row, not column 0's target"
    );

    rt.update(|s, tx| {
        super::super::view::drag_over(&v.sh, s, Some(at_350), tx);
    });
    assert_eq!(rt.state.columns.drop_column(), Some(1));

    // Once the spring settles at 500, both x = 350 and x = 420 belong to column 0's first row.
    for _ in 0..30 {
        rt.frame(DT);
    }
    assert_eq!(
        v.drop_target_at(&rt.state, at_420),
        Some((0, Some(col0_first)))
    );

    // Spring column 0 back from 500 to 300: on the first frame its layout width is 300, while
    // it is still drawn wider than 420, so x = 420 still hits column 0's folder row.
    rt.update(|s, tx| {
        s.columns.set_column_width(0, 300.0);
        tx.animated();
        tx.motion(SPRING);
    });
    rt.frame(DT);
    assert_eq!(
        v.drop_target_at(&rt.state, at_420),
        Some((0, Some(col0_first))),
        "first shrinking frame still hits column 0's drawn folder row past its 300 pt target"
    );
}
