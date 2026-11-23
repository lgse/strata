// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::rc::Rc;

use rune::{Cx, El, Env, Runtime, Tx, WheelEvent, col};
use strata_kit::look::Look;

use super::super::*;
use crate::collection::element::{row_key, viewport_path};
use crate::collection::selection::Listing;
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use crate::scroll::{Autoscroll, BarMode};

const W: f32 = 500.0;
const H: f32 = 300.0;
const DT: f64 = 1.0 / 60.0;

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
    width: f32,
    views: Rc<Cell<usize>>,
}

fn listing(depth: u64, count: u64) -> ListSource {
    ListSource::new(
        (0..count)
            .map(|i| {
                ListEntry::new(
                    EntryId::new(depth * 1_000 + i + 1),
                    format!("item-{i:02}.txt"),
                    EntryKind::File,
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
    s.views.set(s.views.get() + 1);
    rune::row()
        .size(800.0, H)
        .child(col().w(s.width).h_full().child(view_of().build(s)))
}

fn runtime(
    width: f32,
    column_count: usize,
    entries_per_col: u64,
    animations: bool,
) -> Runtime<Host> {
    let mut look = Look::fallback(false);
    look.animations = animations;
    let mut columns = ColumnsState::new("t", Autoscroll::new());
    columns.set_bar_mode(BarMode::Always);
    let mut host = Host {
        sources: Sources(Vec::new()),
        columns,
        look,
        width,
        views: Rc::new(Cell::new(0)),
    };
    let mut tx = Tx::default();
    for depth in 0..column_count {
        host.sources.0.push(listing(depth as u64, entries_per_col));
        let source = &host.sources.0[depth];
        host.columns.push(
            Opening::new(source, Listing::Complete),
            Focus::No,
            &host.look,
            &mut tx,
        );
    }
    let mut rt = Runtime::new(host, view, (width.max(800.0), H), Env::new(false, 1.0));
    rt.set_system_reduced_motion(false);
    rt.poll = Some(|s: &mut Host, t, tx| view_of().tick(s, t, tx));
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..24 {
        rt.frame(DT);
    }
}

fn find_rect(rt: &Runtime<Host>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element {key}"))
        .rect
}

#[test]
fn strip_overflows_tracks_horizontal_overflow_and_does_not_rebuild_on_scroll() {
    // One 300 pt column fits inside a 500 pt strip.
    let mut rt = runtime(W, 1, 30, false);
    assert!(
        !rt.state.columns.strip_overflows(),
        "a single 300 pt column fits in 500 pt"
    );

    // Pushing a second 300 pt column (600 pt total) causes horizontal overflow.
    rt.update(|s, tx| {
        s.sources.0.push(listing(1, 30));
        let source = &s.sources.0[1];
        s.columns.push(
            Opening::new(source, Listing::Complete),
            Focus::No,
            &s.look,
            tx,
        );
    });
    settle(&mut rt);
    assert!(
        rt.state.columns.strip_overflows(),
        "two 300 pt columns overflow a 500 pt strip"
    );

    // Scrolling the column vertically while overflowing triggers zero view rebuilds.
    let v0 = rt.state.views.get();
    for _ in 0..10 {
        rt.wheel(150.0, 150.0, -48.0);
        rt.frame(DT);
    }
    assert_eq!(
        rt.state.views.get(),
        v0,
        "vertical scrolling while strip overflows must not rebuild the view"
    );
    assert!(
        !rt.state.columns.sync_strip_overflow(&rt.state.look),
        "steady overflow reports no transition"
    );

    // Scrolling the strip horizontally does not flip strip_overflows.
    rt.wheel_event(WheelEvent::new(-40.0, 0.0).at(250.0, 150.0));
    rt.frame(DT);
    assert!(rt.state.columns.strip_offset() > 0.0);
    assert!(
        !rt.state.columns.sync_strip_overflow(&rt.state.look),
        "horizontal scroll frames report no overflow transition"
    );

    rt.update(|s, _| s.width = 700.0);
    settle(&mut rt);
    assert!(
        !rt.state.columns.strip_overflows(),
        "widening the strip past total column width clears overflow"
    );
    rt.update(|s, _| s.width = W);
    settle(&mut rt);
    assert!(
        rt.state.columns.strip_overflows(),
        "narrowing the strip below total column width restores overflow"
    );

    // Truncating back to one column clears strip_overflows.
    rt.update(|s, tx| {
        let look = s.look.clone();
        s.columns.truncate(1, &look, tx);
        s.sources.0.truncate(1);
    });
    settle(&mut rt);
    assert!(
        !rt.state.columns.strip_overflows(),
        "truncating back to one column clears horizontal overflow"
    );
}

#[test]
fn strip_overflows_clears_after_animated_truncate_finishes() {
    let mut rt = runtime(W, 2, 10, true);
    assert!(rt.state.columns.strip_overflows());

    rt.update(|s, tx| {
        let look = s.look.clone();
        s.columns.truncate(1, &look, tx);
        s.sources.0.truncate(1);
    });
    for _ in 0..40 {
        rt.frame(DT);
    }
    assert!(
        !rt.state.columns.strip_overflows(),
        "animated truncate clears strip_overflows once the exiting column settles"
    );
}

#[test]
fn bottom_entry_and_column_scrollbar_stay_interactive_when_strip_overflows() {
    let count = 40u64;
    let mut rt = runtime(W, 3, count, false);
    assert!(rt.state.columns.strip_overflows());
    let col_key = rt.state.columns.column_key(0).expect("column 0");
    let vp_key = viewport_path(&col_key);

    // Scroll column 0 all the way to its vertical bottom.
    let max_y = rt.state.columns.column(0).expect("col 0").feed.get().max[1];
    assert!(max_y > 0.0, "column 0 must overflow vertically");
    rt.update(|_, tx| {
        tx.scroll_to_instant(vp_key.clone(), 0.0, max_y);
    });
    settle(&mut rt);

    let last_id = EntryId::new(count);
    let last_rect = find_rect(&rt, &row_key(last_id));

    let click_x = last_rect[0] + last_rect[2] / 2.0;
    let click_y = last_rect[1] + last_rect[3] - 2.0;
    let strip_before = rt.state.columns.strip_offset();
    rt.pointer_down(click_x, click_y);
    rt.frame(DT);
    rt.pointer_up(click_x, click_y);
    settle(&mut rt);

    assert!(
        rt.state
            .columns
            .column(0)
            .expect("col 0")
            .collection
            .selection
            .contains(&rt.state.sources.0[0], last_id),
        "clicking the bottom edge of the last item selects it"
    );
    assert_eq!(
        rt.state.columns.strip_offset(),
        strip_before,
        "clicking the last item must not hit the horizontal strip scrollbar"
    );

    // Click near the bottom of column 0's vertical scrollbar: must NOT jump the horizontal strip.
    let vbar = find_rect(&rt, &format!("{col_key}-bar"));
    let vbar_x = vbar[0] + vbar[2] / 2.0;
    let vbar_y = vbar[1] + vbar[3] - 2.0;
    rt.pointer_down(vbar_x, vbar_y);
    rt.frame(DT);
    rt.pointer_up(vbar_x, vbar_y);
    settle(&mut rt);

    assert_eq!(
        rt.state.columns.strip_offset(),
        strip_before,
        "pressing the bottom of a column's vertical bar must not hit the strip scrollbar"
    );

    let strip_bar = find_rect(&rt, &format!("{}-bar", keys::strip("t")));
    let hbar_x = strip_bar[0] + strip_bar[2] - 20.0;
    let hbar_y = strip_bar[1] + strip_bar[3] / 2.0;
    rt.pointer_down(hbar_x, hbar_y);
    rt.frame(DT);
    rt.pointer_up(hbar_x, hbar_y);
    settle(&mut rt);

    assert!(
        rt.state.columns.strip_offset() > strip_before,
        "clicking the horizontal strip bar trough still scrolls the strip"
    );
}
