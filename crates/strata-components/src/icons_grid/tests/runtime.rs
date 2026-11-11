//! The grid in a headless runtime: what lands (details, thumbnails) rebuilds only what shows
//! it, two grids follow one size, a scroll offset comes back only at its grid's width, and a
//! 4K screen at scale 2 settles its thumbnail queue (C25's B1, at the grid).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Cx, El, Env, Runtime, Tx, col, row};
use strata_kit::look::Look;

use crate::collection::element::{Collection, CollectionState};
use crate::collection::selection::{Listing, Position, ScrollMemo};
use crate::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource, ThumbKey};
use crate::icons_grid::{IconsGrid, IconsGridState, IconsParts, SizeCommand, SizeRequest};
use crate::thumbnails::{Outcome, ThumbnailStore, ViewerId};

const DT: f64 = 1.0 / 60.0;
const SCOPE: &str = "grid";

struct Host {
    src: ListSource,
    coll: CollectionState,
    grid: IconsGridState,
    look: Look,
    store: ThumbnailStore,
    viewer: ViewerId,
    size: f32,
    details: HashMap<EntryId, Arc<str>>,
    /// Details read while building tiles.
    detail_reads: Rc<Cell<usize>>,
    views: Rc<Cell<usize>>,
    w: f32,
    h: f32,
    /// A second grid over the same size, beside the first.
    second: Option<(CollectionState, IconsGridState)>,
}

fn source(n: usize) -> ListSource {
    ListSource::new(
        (0..n)
            .map(|i| {
                let mut e = ListEntry::new(
                    EntryId::new(i as u64 + 1),
                    format!("img-{i}.jpg"),
                    EntryKind::File,
                    "file_image",
                );
                e.thumbnail = Some(ThumbKey::new(10_000 + i as u64));
                e
            })
            .collect(),
    )
}

fn host(n: usize, w: f32, h: f32) -> Host {
    let mut store = ThumbnailStore::default();
    let viewer = store.viewer();
    Host {
        src: source(n),
        coll: CollectionState::new(),
        grid: IconsGridState::new(),
        look: Look::fallback(false),
        store,
        viewer,
        size: 64.0,
        details: HashMap::new(),
        detail_reads: Rc::default(),
        views: Rc::default(),
        w,
        h,
        second: None,
    }
}

fn base(_: &Host) -> Collection<Host> {
    Collection::new(
        SCOPE,
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
}

fn grid(s: &Host) -> IconsGrid<Host> {
    let reads = s.detail_reads.clone();
    IconsGrid::new(
        SCOPE,
        base,
        |s: &Host| IconsParts {
            source: &s.src,
            collection: &s.coll,
            grid: &s.grid,
            size: s.size,
            listing: 1,
            look: &s.look,
            thumbs: Some((&s.store, s.viewer)),
        },
        |s: &mut Host| &mut s.grid,
    )
    .thumbs_mut(|s: &mut Host| Some((&mut s.store, s.viewer, &s.src as &dyn EntrySource)))
    .details(
        |s: &Host, rcx| {
            s.details
                .get(&rcx.entry.id)
                .map_or(0, |d| d.len() as u64 + 1)
        },
        move |s: &Host, rcx| {
            reads.set(reads.get() + 1);
            s.details.get(&rcx.entry.id).cloned()
        },
    )
    .on_size(|s: &mut Host, r, _| {
        if let SizeRequest::Step { to, .. } = r {
            s.size = to;
        }
    })
}

/// A second grid over the same source and size, with its own states.
fn second(_: &Host) -> IconsGrid<Host> {
    IconsGrid::new(
        "second",
        |_: &Host| {
            Collection::new(
                "second",
                |s: &Host| (&s.src, &s.second.as_ref().expect("second").0),
                |s: &mut Host| (&s.src, &mut s.second.as_mut().expect("second").0),
                |s: &Host| &s.look,
                |_: &Host| 1,
            )
        },
        |s: &Host| {
            let (coll, grid) = s.second.as_ref().expect("second");
            IconsParts {
                source: &s.src,
                collection: coll,
                grid,
                size: s.size,
                listing: 1,
                look: &s.look,
                thumbs: None,
            }
        },
        |s: &mut Host| &mut s.second.as_mut().expect("second").1,
    )
}

fn view(s: &Host, cx: &mut Cx<Host>) -> El {
    s.views.set(s.views.get() + 1);
    let mut r = row().size(s.w, s.h).child(
        col()
            .w(s.w / if s.second.is_some() { 2.0 } else { 1.0 })
            .h_full()
            .child(grid(s).scale(cx.env().scale).build(s)),
    );
    if s.second.is_some() {
        r = r.child(col().w(s.w / 2.0).h_full().child(second(s).build(s)));
    }
    r
}

fn runtime(h: Host, scale: f32) -> Runtime<Host> {
    let (w, hh) = (h.w, h.h);
    let mut rt = Runtime::new(h, view, (w, hh), Env::new(false, scale));
    for _ in 0..4 {
        rt.frame(DT);
    }
    rt
}

/// A worker that finishes every request at once, as a host's poll would hand them over.
fn serve(s: &mut Host) -> usize {
    let mut n = 0;
    while let Some(r) = s.store.pop(0.0) {
        let px = r.step.px();
        s.store.complete(
            r.ticket,
            Outcome::Ready {
                width: px,
                height: px,
                rgba: vec![200; (px * px * 4) as usize],
            },
            0.0,
        );
        n += 1;
    }
    n
}

#[test]
fn a_details_line_landing_rebuilds_its_tile_alone() {
    let mut rt = runtime(host(400, 900.0, 600.0), 1.0);
    let shown = rt.state.coll.visible_range();
    assert!(shown.len() > 8);
    let reads = rt.state.detail_reads.clone();
    reads.set(0);
    let views = rt.state.views.get();
    rt.update(|s: &mut Host, _: &mut Tx| {
        let id = s.src.id_at(shown.start + 3);
        s.details.insert(id, "1.2 MB".into());
    });
    rt.frame(DT);
    assert_eq!(
        rt.state.views.get(),
        views + 1,
        "the host's change commits once"
    );
    assert_eq!(
        reads.get(),
        1,
        "and builds the one tile whose details landed"
    );
}

#[test]
fn thumbnails_landing_run_no_view_and_rebuild_no_tile() {
    let mut rt = runtime(host(400, 900.0, 600.0), 1.0);
    let reads = rt.state.detail_reads.clone();
    reads.set(0);
    let views = rt.state.views.get();
    // Handed over as a host's poll does, which returns false: nothing the view reads changed.
    assert!(serve(&mut rt.state) > 0);
    for _ in 0..10 {
        rt.frame(DT);
    }
    assert_eq!(rt.state.views.get(), views, "no view run");
    assert_eq!(reads.get(), 0, "no tile rebuilt");
}

#[test]
fn two_grids_over_one_size_both_follow_a_step() {
    let mut h = host(200, 1200.0, 600.0);
    h.second = Some((CollectionState::new(), IconsGridState::new()));
    let mut rt = runtime(h, 1.0);
    let per = |rt: &Runtime<Host>| {
        (
            rt.state.coll.per_line(),
            rt.state.second.as_ref().expect("second").0.per_line(),
        )
    };
    let before = per(&rt);
    rt.update(|s: &mut Host, tx: &mut Tx| grid(s).apply_size(s, SizeCommand::Step(6), tx));
    for _ in 0..4 {
        rt.frame(DT);
    }
    let after = per(&rt);
    assert!(
        after.0 < before.0 && after.1 < before.1,
        "{before:?} → {after:?}"
    );
    assert_eq!(after.0, after.1, "both at the one size");
}

#[test]
fn a_scroll_offset_comes_back_only_at_its_grids_width() {
    let mut rt = runtime(host(2_000, 900.0, 600.0), 1.0);
    let key = |s: &Host| IconsGrid::<Host>::layout_key(s.coll.viewport().w, s.size, &s.look);
    let layout = key(&rt.state);
    rt.update(|s: &mut Host, _: &mut Tx| s.coll.set_layout_key(layout));
    // Left at a cursor near the top, scrolled well past it.
    rt.update(|s: &mut Host, _: &mut Tx| {
        s.coll.selection.select_only(&s.src, 2);
    });
    let at = 5_000.0;
    let position = |s: &Host, l: u64| -> Position {
        s.coll
            .selection
            .position(&s.src)
            .with_scroll(ScrollMemo::new(l, at))
    };
    let back = position(&rt.state, layout);
    rt.update(|s: &mut Host, tx: &mut Tx| {
        s.coll
            .enter(SCOPE, &s.src, Some(&back), Listing::Complete, tx);
    });
    for _ in 0..10 {
        rt.frame(DT);
    }
    assert_eq!(
        rt.state.coll.scroll_offset(),
        at,
        "the same width: the offset"
    );
    // Narrowed, the grid wraps otherwise: its key differs, and the cursor comes into view
    // instead of the offset.
    rt.update(|s: &mut Host, _: &mut Tx| s.w = 400.0);
    for _ in 0..10 {
        rt.frame(DT);
    }
    let other = key(&rt.state);
    assert_ne!(other, layout, "another count a row");
    rt.update(|s: &mut Host, tx: &mut Tx| {
        s.coll.set_layout_key(other);
        s.coll
            .enter(SCOPE, &s.src, Some(&back), Listing::Complete, tx);
    });
    for _ in 0..10 {
        rt.frame(DT);
    }
    assert!(
        rt.state.coll.visible_range().contains(&2),
        "the cursor shows at {}",
        rt.state.coll.scroll_offset()
    );
}

/// C25's review item B1, at the grid: a 3840 × 2160 window at scale 2 with 72 pt tiles shows
/// about 60 MiB of thumbnails against the default 64 MiB budget. Served as it asks, the store
/// settles: after the screen and what margin fits load, it asks for nothing more.
#[test]
fn a_4k_grid_at_scale_2_settles_its_thumbnail_queue() {
    let mut h = host(20_000, 1920.0, 1080.0);
    h.size = 72.0;
    let mut rt = runtime(h, 2.0);
    let mut served = Vec::new();
    for _ in 0..30 {
        served.push(serve(&mut rt.state));
        rt.frame(DT);
    }
    assert!(served[0] > 0);
    assert!(
        served[10..].iter().all(|&n| n == 0),
        "nothing asked again once settled: {served:?}"
    );
    let st = rt.state.store.stats();
    assert!(st.bytes <= st.budget, "{} of {}", st.bytes, st.budget);
}

/// The grid syncs the store with its count a row (C25's `Viewport::per_line`), so the margin it
/// loads past the viewport is whole lines of tiles, as the grid draws them: served as it asks,
/// the loaded thumbnails end on a line's last tile, a line or more past the viewport.
#[test]
fn the_thumbnail_margin_is_whole_lines_of_tiles() {
    let mut rt = runtime(host(2_000, 900.0, 600.0), 1.0);
    for _ in 0..10 {
        serve(&mut rt.state);
        rt.frame(DT);
    }
    let s = &rt.state;
    let per = s.coll.per_line();
    assert!(per > 1);
    let shown = s.coll.visible_range();
    let loaded: Vec<usize> = (0..s.src.len())
        .filter(|&i| {
            let key = s.src.entry(i).thumbnail.expect("every entry has one");
            matches!(s.store.state(key), crate::thumbnails::State::Ready { .. })
        })
        .collect();
    let end = loaded.last().copied().expect("some loaded") + 1;
    assert_eq!(loaded.len(), end, "loaded from the top without a hole");
    assert!(
        end >= shown.end + per,
        "a line or more past the viewport: {end} vs {shown:?}"
    );
    assert_eq!(end % per, 0, "whole lines of {per}: {end}");
}
