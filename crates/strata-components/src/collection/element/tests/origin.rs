// SPDX-License-Identifier: MIT

use rune::{Drag, Tx};

use super::*;

const STRIP: f32 = 60.0;

/// The fixture's collection, allowing one selected entry when the host says so.
fn origin_collection(s: &Host) -> Collection<Host> {
    let single = crate::collection::pointer::PointerSettings::default().multiple(false);
    let c = collection(s);
    if s.single { c.settings(single) } else { c }
}

/// A blank strip left of the collection whose drags start the collection's band.
fn with_origin(s: &Host, _: &mut Cx<Host>) -> El {
    let strip = rune::el().key("origin").w(STRIP).h_full().on_drag(
        |s: &mut Host, d: &mut Drag, tx: &mut Tx| {
            origin_collection(s).origin_drag(s, (d, true), tx);
        },
    );
    rune::row().size(W, H).child(strip).child(
        col()
            .w(W - STRIP)
            .h_full()
            .child(origin_collection(s).build(s)),
    )
}

/// What a drag from the strip across the first rows leaves selected.
fn band_from_the_strip(single: bool) -> Vec<EntryId> {
    let mut host = Host::new(20);
    host.single = single;
    host.coll.controllers.pointer =
        Some(Box::new(crate::collection::pointer::PointerIntent::new()));
    host.coll.controllers.marquee = Some(Box::new(crate::collection::marquee::Marquee::new()));
    let mut rt = Runtime::new(host, with_origin, (W, H), Env::new(false, 1.0));
    settle(&mut rt);
    rt.state.coll.selection.select_only(&rt.state.src, 5);
    let row_h = rt.state.look.row.list;
    let (x, y) = (STRIP / 2.0, 2.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    for k in 1..=6 {
        rt.pointer_move(x + 20.0 * k as f32, y + row_h * 0.5 * k as f32);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(STRIP + 60.0, y + row_h * 2.5);
    settle(&mut rt);
    let s = &rt.state;
    s.coll.selection.selected_ids(&s.src)
}

#[test]
fn a_band_from_outside_the_viewport_selects_only_where_more_than_one_entry_may_be() {
    // portal/chooser: a single-selection chooser selects one entry at a time, so a drag from blank space draws no band.
    assert_eq!(
        band_from_the_strip(false),
        [id(0), id(1), id(2)],
        "the band selects the rows it crosses"
    );
    assert_eq!(
        band_from_the_strip(true),
        [],
        "the press clears, and no band follows"
    );
}
